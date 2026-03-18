use std::time::Duration;
use axum::{
    extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade},
    response::Response,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};

/// Handle a WebSocket upgrade request and proxy frames bidirectionally to
/// the backend WebSocket server at `backend_url`.
///
/// `connect_timeout_secs` is applied to the initial backend connection attempt.
/// The caller is responsible for passing a URL that came from `RouteConfig`
/// (config-defined), never from request data (SSRF mitigation).
pub fn handle_websocket_upgrade(
    upgrade: WebSocketUpgrade,
    backend_url: String,
    connect_timeout_secs: u64,
) -> Response {
    upgrade.on_upgrade(move |client_ws| {
        proxy_websocket(client_ws, backend_url, connect_timeout_secs)
    })
}

async fn proxy_websocket(client_ws: WebSocket, backend_url: String, connect_timeout_secs: u64) {
    let backend_url = match normalize_ws_url(&backend_url) {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("WebSocket proxy: invalid backend URL '{}': {}", backend_url, e);
            return;
        }
    };

    let connect_result = tokio::time::timeout(
        Duration::from_secs(connect_timeout_secs),
        connect_async(&backend_url),
    )
    .await;

    let (backend_ws, _) = match connect_result {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            tracing::error!("WebSocket proxy: backend connect to '{}' failed: {}", backend_url, e);
            return;
        }
        Err(_) => {
            tracing::error!(
                "WebSocket proxy: backend connect to '{}' timed out after {}s",
                backend_url, connect_timeout_secs
            );
            return;
        }
    };

    tracing::debug!("WebSocket proxy: connected to backend {}", backend_url);

    let (mut client_sink, mut client_stream) = client_ws.split();
    let (mut backend_sink, mut backend_stream) = backend_ws.split();

    // Client → backend
    let c2b = async {
        while let Some(msg_result) = client_stream.next().await {
            match msg_result {
                Ok(msg) => {
                    if matches!(msg, AxumMessage::Close(_)) {
                        break;
                    }
                    if let Some(t_msg) = axum_to_tungstenite(msg) {
                        if backend_sink.send(t_msg).await.is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("WebSocket proxy: client stream error: {}", e);
                    break;
                }
            }
        }
        let _ = backend_sink.close().await;
    };

    // Backend → client
    let b2c = async {
        while let Some(msg_result) = backend_stream.next().await {
            match msg_result {
                Ok(msg) => {
                    if matches!(msg, TungsteniteMessage::Close(_)) {
                        break;
                    }
                    if let Some(a_msg) = tungstenite_to_axum(msg) {
                        if client_sink.send(a_msg).await.is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!("WebSocket proxy: backend stream error: {}", e);
                    break;
                }
            }
        }
        let _ = client_sink.close().await;
    };

    // Run both directions concurrently; stop when either side closes.
    tokio::select! {
        _ = c2b => {},
        _ = b2c => {},
    }

    tracing::debug!("WebSocket proxy: session closed for {}", backend_url);
}

/// Convert an axum WebSocket message to a tungstenite message.
fn axum_to_tungstenite(msg: AxumMessage) -> Option<TungsteniteMessage> {
    match msg {
        AxumMessage::Text(s) => Some(TungsteniteMessage::Text(s.to_string())),
        AxumMessage::Binary(b) => Some(TungsteniteMessage::Binary(b.to_vec())),
        AxumMessage::Ping(b) => Some(TungsteniteMessage::Ping(b.to_vec())),
        AxumMessage::Pong(b) => Some(TungsteniteMessage::Pong(b.to_vec())),
        AxumMessage::Close(_) => None,
    }
}

/// Convert a tungstenite message to an axum WebSocket message.
fn tungstenite_to_axum(msg: TungsteniteMessage) -> Option<AxumMessage> {
    match msg {
        TungsteniteMessage::Text(s) => Some(AxumMessage::Text(s.into())),
        TungsteniteMessage::Binary(b) => Some(AxumMessage::Binary(b.into())),
        TungsteniteMessage::Ping(b) => Some(AxumMessage::Ping(b.into())),
        TungsteniteMessage::Pong(b) => Some(AxumMessage::Pong(b.into())),
        TungsteniteMessage::Close(_) => {
            // Propagate a plain close without a frame to avoid version-specific
            // type differences between tungstenite 0.24 and axum's internal 0.28.
            Some(AxumMessage::Close(None))
        }
        TungsteniteMessage::Frame(_) => None,
    }
}

/// Accept `ws://`, `wss://`, `http://`, or `https://` and normalise to the
/// `ws://` / `wss://` form that `connect_async` expects. Plain `host:port`
/// strings are prefixed with `ws://`.
fn normalize_ws_url(url: &str) -> Result<String, String> {
    if url.starts_with("ws://") || url.starts_with("wss://") {
        Ok(url.to_string())
    } else if url.starts_with("http://") {
        Ok(url.replacen("http://", "ws://", 1))
    } else if url.starts_with("https://") {
        Ok(url.replacen("https://", "wss://", 1))
    } else if !url.contains("://") {
        // Bare host:port
        Ok(format!("ws://{}", url))
    } else {
        Err(format!(
            "WebSocket backend URL must use ws://, wss://, http://, or https:// scheme"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ws_url_passthrough() {
        assert_eq!(normalize_ws_url("ws://host:8080/path").unwrap(), "ws://host:8080/path");
        assert_eq!(normalize_ws_url("wss://host:443/path").unwrap(), "wss://host:443/path");
    }

    #[test]
    fn normalize_ws_url_from_http() {
        assert_eq!(normalize_ws_url("http://host:8080").unwrap(), "ws://host:8080");
        assert_eq!(normalize_ws_url("https://host:443").unwrap(), "wss://host:443");
    }

    #[test]
    fn normalize_ws_url_bare_host() {
        assert_eq!(normalize_ws_url("host:8080").unwrap(), "ws://host:8080");
    }

    #[test]
    fn normalize_ws_url_rejects_ftp() {
        assert!(normalize_ws_url("ftp://host/path").is_err());
    }
}
