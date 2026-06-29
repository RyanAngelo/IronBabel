use std::time::Duration;
use axum::{
    extract::ws::{CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket, WebSocketUpgrade},
    response::Response,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::protocol::{CloseFrame as TungCloseFrame, WebSocketConfig},
    tungstenite::Message as TungsteniteMessage,
};

/// Maximum size of a single complete WebSocket message proxied in either
/// direction. Caps memory use per connection (mirrors the 10 MB HTTP body
/// limit) and prevents a malicious client or backend from exhausting memory
/// with an unbounded frame.
const MAX_WS_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
/// Maximum size of a single WebSocket frame.
const MAX_WS_FRAME_SIZE: usize = 10 * 1024 * 1024;

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
    upgrade
        .max_message_size(MAX_WS_MESSAGE_SIZE)
        .max_frame_size(MAX_WS_FRAME_SIZE)
        .on_upgrade(move |client_ws| {
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

    // Cap message/frame size on the backend connection too, so an oversized
    // frame from the backend cannot exhaust memory on its way to the client.
    let ws_config = WebSocketConfig {
        max_message_size: Some(MAX_WS_MESSAGE_SIZE),
        max_frame_size: Some(MAX_WS_FRAME_SIZE),
        ..Default::default()
    };

    let connect_result = tokio::time::timeout(
        Duration::from_secs(connect_timeout_secs),
        connect_async_with_config(&backend_url, Some(ws_config), false),
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
                    // Forward the close frame (with its code/reason) to the
                    // backend before stopping, so the peer learns *why* the
                    // session ended rather than just seeing a bare close.
                    let is_close = matches!(msg, AxumMessage::Close(_));
                    if let Some(t_msg) = axum_to_tungstenite(msg) {
                        if backend_sink.send(t_msg).await.is_err() {
                            break;
                        }
                    }
                    if is_close {
                        break;
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
                    let is_close = matches!(msg, TungsteniteMessage::Close(_));
                    if let Some(a_msg) = tungstenite_to_axum(msg) {
                        if client_sink.send(a_msg).await.is_err() {
                            break;
                        }
                    }
                    if is_close {
                        break;
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
        // Preserve the close code and reason across the protocol boundary.
        AxumMessage::Close(frame) => Some(TungsteniteMessage::Close(frame.map(|f| {
            TungCloseFrame {
                code: f.code.into(),
                reason: std::borrow::Cow::Owned(f.reason.as_str().to_string()),
            }
        }))),
    }
}

/// Convert a tungstenite message to an axum WebSocket message.
fn tungstenite_to_axum(msg: TungsteniteMessage) -> Option<AxumMessage> {
    match msg {
        TungsteniteMessage::Text(s) => Some(AxumMessage::Text(s.into())),
        TungsteniteMessage::Binary(b) => Some(AxumMessage::Binary(b.into())),
        TungsteniteMessage::Ping(b) => Some(AxumMessage::Ping(b.into())),
        TungsteniteMessage::Pong(b) => Some(AxumMessage::Pong(b.into())),
        // Preserve the close code and reason across the protocol boundary.
        TungsteniteMessage::Close(frame) => Some(AxumMessage::Close(frame.map(|f| {
            AxumCloseFrame {
                code: u16::from(f.code),
                reason: f.reason.to_string().into(),
            }
        }))),
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
        Err("WebSocket backend URL must use ws://, wss://, http://, or https:// scheme".to_string())
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
