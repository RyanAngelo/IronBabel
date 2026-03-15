use std::time::Duration;

use zeromq::prelude::*;

use crate::config::ZmqPullListenerConfig;
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert an address to the `tcp://host:port` form that the zeromq crate
/// expects. Accepts `zmq://host:port`, plain `host:port`, or an already-valid
/// `tcp://host:port` address.
fn to_tcp_addr(addr: &str) -> String {
    if addr.starts_with("zmq://") {
        addr.replacen("zmq://", "tcp://", 1)
    } else if addr.starts_with("tcp://") {
        addr.to_string()
    } else {
        format!("tcp://{}", addr)
    }
}

/// Flatten all frames of a `ZmqMessage` into a single `Vec<u8>`.
/// For well-behaved single-frame messages this is a direct copy; for
/// multi-frame messages frames are concatenated in order.
fn msg_to_bytes(msg: zeromq::ZmqMessage) -> Vec<u8> {
    msg.into_vec().into_iter().flat_map(|frame| frame.to_vec()).collect()
}

// ---------------------------------------------------------------------------
// ZmqGateway — HTTP → ZMQ (req_rep and push patterns)
// ---------------------------------------------------------------------------

/// Zero-cost gateway handle. All methods create a fresh socket per call so
/// concurrent requests never share socket state.
pub struct ZmqGateway;

impl ZmqGateway {
    pub fn new() -> Self {
        Self
    }

    /// **REQ/REP** — send `body` to a ZMQ REP upstream and return the reply.
    ///
    /// Opens a REQ socket, connects to `target` (`zmq://host:port`), sends
    /// `body` as a single frame, waits up to `timeout_secs` for a REP frame,
    /// and returns the reply bytes.
    pub async fn forward_req_rep(
        &self,
        target: &str,
        body: Vec<u8>,
        timeout_secs: u64,
    ) -> Result<Vec<u8>> {
        let addr = to_tcp_addr(target);

        let mut socket = zeromq::ReqSocket::new();
        socket
            .connect(&addr)
            .await
            .map_err(|e| Error::Protocol(format!("ZMQ connect failed ({}): {}", addr, e)))?;

        socket
            .send(zeromq::ZmqMessage::from(body))
            .await
            .map_err(|e| Error::Protocol(format!("ZMQ send failed: {}", e)))?;

        let reply = tokio::time::timeout(Duration::from_secs(timeout_secs), socket.recv())
            .await
            .map_err(|_| {
                Error::Protocol(format!(
                    "ZMQ REQ/REP timeout after {}s waiting for reply from {}",
                    timeout_secs, addr
                ))
            })?
            .map_err(|e| Error::Protocol(format!("ZMQ recv failed: {}", e)))?;

        Ok(msg_to_bytes(reply))
    }

    /// **PUSH** — send `body` to a ZMQ PULL upstream, fire-and-forget.
    ///
    /// Opens a PUSH socket, connects to `target`, sends `body` as a single
    /// frame, and returns immediately. The caller should respond with 202.
    pub async fn forward_push(&self, target: &str, body: Vec<u8>) -> Result<()> {
        let addr = to_tcp_addr(target);

        let mut socket = zeromq::PushSocket::new();
        socket
            .connect(&addr)
            .await
            .map_err(|e| Error::Protocol(format!("ZMQ connect failed ({}): {}", addr, e)))?;

        socket
            .send(zeromq::ZmqMessage::from(body))
            .await
            .map_err(|e| Error::Protocol(format!("ZMQ send failed: {}", e)))?;

        Ok(())
    }

    /// **PUB/SUB** — publish `body` to a ZMQ subscriber, fire-and-forget.
    ///
    /// Opens a PUB socket, connects to `target`, and sends a single frame
    /// containing the optional `topic` prefix followed by `body`. Subscribers
    /// filter by the topic prefix via `set_subscribe`. Returns immediately;
    /// the caller should respond with 202.
    pub async fn forward_pub(
        &self,
        target: &str,
        body: Vec<u8>,
        topic: Option<&str>,
    ) -> Result<()> {
        let addr = to_tcp_addr(target);

        let mut socket = zeromq::PubSocket::new();
        socket
            .connect(&addr)
            .await
            .map_err(|e| Error::Protocol(format!("ZMQ connect failed ({}): {}", addr, e)))?;

        // Prefix the message with the topic bytes so subscribers can filter.
        // Convention: [topic_bytes || body_bytes] in a single frame.
        let mut message = topic.unwrap_or("").as_bytes().to_vec();
        message.extend_from_slice(&body);

        socket
            .send(zeromq::ZmqMessage::from(message))
            .await
            .map_err(|e| Error::Protocol(format!("ZMQ send failed: {}", e)))?;

        Ok(())
    }
}

impl Default for ZmqGateway {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// run_pull_listener — ZMQ → HTTP (pull pattern, background task)
// ---------------------------------------------------------------------------

/// Background task: bind a ZMQ PULL socket at `config.listen`, receive frames
/// indefinitely, and forward each one as an HTTP POST to `config.target`.
///
/// Spawned once per `zmq_listeners` entry during gateway startup.
/// The task exits only on socket error; in practice it runs until the process
/// terminates.
pub async fn run_pull_listener(config: ZmqPullListenerConfig) {
    let bind_addr = to_tcp_addr(&config.bind);
    let http_target = config.forward_to.clone();

    let mut socket = zeromq::PullSocket::new();
    match socket.bind(&bind_addr).await {
        Ok(_) => tracing::info!("ZMQ PULL listener bound on {}", bind_addr),
        Err(e) => {
            tracing::error!("ZMQ PULL listener failed to bind {}: {}", bind_addr, e);
            return;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    loop {
        match socket.recv().await {
            Ok(msg) => {
                let body = msg_to_bytes(msg);
                let target = http_target.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    match client
                        .post(&target)
                        .header("content-type", "application/octet-stream")
                        .header("x-zmq-source", "ironbabel-pull-listener")
                        .body(body)
                        .send()
                        .await
                    {
                        Ok(resp) => tracing::debug!(
                            "ZMQ→HTTP forwarded to {} → {}",
                            target,
                            resp.status()
                        ),
                        Err(e) => tracing::warn!("ZMQ→HTTP forward to {} failed: {}", target, e),
                    }
                });
            }
            Err(e) => {
                tracing::error!("ZMQ PULL listener recv error on {}: {}", bind_addr, e);
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::time::sleep;

    // ── helpers ─────────────────────────────────────────────────────────────

    /// Bind a REP echo server on `port`. Returns a handle that can be aborted.
    async fn spawn_rep_echo(port: u16) -> tokio::task::JoinHandle<()> {
        let mut socket = zeromq::RepSocket::new();
        socket
            .bind(&format!("tcp://127.0.0.1:{}", port))
            .await
            .expect("rep bind");
        tokio::spawn(async move {
            loop {
                match socket.recv().await {
                    Ok(msg) => {
                        if socket.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    }

    /// Bind a REP server that responds with a fixed payload.
    async fn spawn_rep_fixed(port: u16, reply: Vec<u8>) -> tokio::task::JoinHandle<()> {
        let mut socket = zeromq::RepSocket::new();
        socket
            .bind(&format!("tcp://127.0.0.1:{}", port))
            .await
            .expect("rep bind");
        tokio::spawn(async move {
            if let Ok(_msg) = socket.recv().await {
                let _ = socket.send(zeromq::ZmqMessage::from(reply)).await;
            }
        })
    }

    /// Bind a REP server that receives but never replies (for timeout tests).
    async fn spawn_rep_silent(port: u16) -> tokio::task::JoinHandle<()> {
        let mut socket = zeromq::RepSocket::new();
        socket
            .bind(&format!("tcp://127.0.0.1:{}", port))
            .await
            .expect("rep bind");
        tokio::spawn(async move {
            // Receive and discard — never send a reply
            let _ = socket.recv().await;
            sleep(Duration::from_secs(60)).await;
        })
    }

    /// Bind a PULL receiver. Returns a handle and a shared vec of received frames.
    async fn spawn_pull_receiver(port: u16) -> (tokio::task::JoinHandle<()>, Arc<Mutex<Vec<Vec<u8>>>>) {
        let received: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);

        let mut socket = zeromq::PullSocket::new();
        socket
            .bind(&format!("tcp://127.0.0.1:{}", port))
            .await
            .expect("pull bind");

        let handle = tokio::spawn(async move {
            while let Ok(msg) = socket.recv().await {
                let bytes = msg_to_bytes(msg);
                received_clone.lock().await.push(bytes);
            }
        });

        (handle, received)
    }

    // ── addr conversion ──────────────────────────────────────────────────────

    #[test]
    fn zmq_to_tcp_addr() {
        assert_eq!(to_tcp_addr("zmq://127.0.0.1:5555"), "tcp://127.0.0.1:5555");
        assert_eq!(to_tcp_addr("zmq://localhost:9999"), "tcp://localhost:9999");
        // Non-zmq URLs are returned unchanged (should never reach here in practice)
        assert_eq!(to_tcp_addr("tcp://127.0.0.1:5555"), "tcp://127.0.0.1:5555");
    }

    // ── req/rep ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn req_rep_echo_round_trip() {
        let port = 15_501u16;
        let handle = spawn_rep_echo(port).await;
        sleep(Duration::from_millis(50)).await;

        let gw = ZmqGateway::new();
        let body = b"hello world".to_vec();
        let result = gw
            .forward_req_rep(&format!("zmq://127.0.0.1:{}", port), body.clone(), 5)
            .await
            .unwrap();

        assert_eq!(result, body);
        handle.abort();
    }

    #[tokio::test]
    async fn req_rep_returns_upstream_reply() {
        let port = 15_502u16;
        let fixed = b"upstream response payload".to_vec();
        let handle = spawn_rep_fixed(port, fixed.clone()).await;
        sleep(Duration::from_millis(50)).await;

        let gw = ZmqGateway::new();
        let result = gw
            .forward_req_rep(&format!("zmq://127.0.0.1:{}", port), b"request".to_vec(), 5)
            .await
            .unwrap();

        assert_eq!(result, fixed);
        handle.abort();
    }

    #[tokio::test]
    async fn req_rep_json_body_round_trip() {
        let port = 15_503u16;
        let handle = spawn_rep_echo(port).await;
        sleep(Duration::from_millis(50)).await;

        let payload = serde_json::json!({"order_id": "ord-001", "amount": 99.99});
        let body = serde_json::to_vec(&payload).unwrap();

        let gw = ZmqGateway::new();
        let result = gw
            .forward_req_rep(&format!("zmq://127.0.0.1:{}", port), body.clone(), 5)
            .await
            .unwrap();

        let echoed: serde_json::Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(echoed, payload);
        handle.abort();
    }

    #[tokio::test]
    async fn req_rep_timeout_on_silent_upstream() {
        let port = 15_504u16;
        let handle = spawn_rep_silent(port).await;
        sleep(Duration::from_millis(50)).await;

        let gw = ZmqGateway::new();
        let err = gw
            .forward_req_rep(&format!("zmq://127.0.0.1:{}", port), b"ping".to_vec(), 1)
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("timeout"),
            "expected timeout error, got: {}",
            err
        );
        handle.abort();
    }

    #[tokio::test]
    async fn req_rep_error_on_unreachable_target() {
        // Nothing listening on this port
        let gw = ZmqGateway::new();
        let result = gw
            .forward_req_rep("zmq://127.0.0.1:19_999", b"ping".to_vec(), 1)
            .await;
        // Should either error on connect or timeout
        assert!(result.is_err());
    }

    // ── push ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn push_delivers_message_to_pull_receiver() {
        let port = 15_505u16;
        let (handle, received) = spawn_pull_receiver(port).await;
        sleep(Duration::from_millis(50)).await;

        let gw = ZmqGateway::new();
        let body = b"event payload".to_vec();
        gw.forward_push(&format!("zmq://127.0.0.1:{}", port), body.clone())
            .await
            .unwrap();

        // Give the receiver task time to process
        sleep(Duration::from_millis(300)).await;

        let msgs = received.lock().await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0], body);
        handle.abort();
    }

    #[tokio::test]
    async fn push_delivers_json_payload() {
        let port = 15_506u16;
        let (handle, received) = spawn_pull_receiver(port).await;
        sleep(Duration::from_millis(50)).await;

        let payload = serde_json::json!({"event": "user.signup", "user_id": "u-123"});
        let body = serde_json::to_vec(&payload).unwrap();

        let gw = ZmqGateway::new();
        gw.forward_push(&format!("zmq://127.0.0.1:{}", port), body.clone())
            .await
            .unwrap();

        sleep(Duration::from_millis(300)).await;

        let msgs = received.lock().await;
        assert_eq!(msgs.len(), 1);
        let received_val: serde_json::Value = serde_json::from_slice(&msgs[0]).unwrap();
        assert_eq!(received_val, payload);
        handle.abort();
    }

    #[tokio::test]
    async fn push_multiple_messages_all_delivered() {
        let port = 15_507u16;
        let (handle, received) = spawn_pull_receiver(port).await;
        sleep(Duration::from_millis(50)).await;

        let gw = ZmqGateway::new();
        for i in 0u8..5 {
            gw.forward_push(
                &format!("zmq://127.0.0.1:{}", port),
                vec![i],
            )
            .await
            .unwrap();
        }

        sleep(Duration::from_millis(400)).await;

        let msgs = received.lock().await;
        assert_eq!(msgs.len(), 5);
        handle.abort();
    }

    // ── msg_to_bytes ─────────────────────────────────────────────────────────

    #[test]
    fn msg_to_bytes_single_frame() {
        let msg = zeromq::ZmqMessage::from(b"hello".to_vec());
        assert_eq!(msg_to_bytes(msg), b"hello");
    }

    #[test]
    fn msg_to_bytes_empty() {
        let msg = zeromq::ZmqMessage::from(Vec::<u8>::new());
        assert_eq!(msg_to_bytes(msg), b"");
    }
}
