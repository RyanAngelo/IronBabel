use async_trait::async_trait;
use serde_json::Value;
use crate::error::{Error, Result};
use super::Protocol;

const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10MB

pub struct HttpProtocol {
    name: String,
}

impl HttpProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "http".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for HttpProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() > MAX_BODY_SIZE {
            return Err(Error::Protocol("body too large".to_string()));
        }
        Ok(data)
    }

    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() > MAX_BODY_SIZE {
            return Err(Error::Protocol("body too large".to_string()));
        }
        Ok(data)
    }
}

/// Strips hop-by-hop headers that must not be forwarded to backends.
/// See RFC 7230 Section 6.1.
pub fn strip_hop_by_hop_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    const HOP_BY_HOP: &[&str] = &[
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
    ];
    headers
        .iter()
        .filter(|(name, _)| {
            let lower = name.to_lowercase();
            !HOP_BY_HOP.contains(&lower.as_str())
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> HttpProtocol {
        HttpProtocol::new(Value::Null).unwrap()
    }

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // --- Protocol name ---

    #[test]
    fn name_is_http() {
        assert_eq!(protocol().name(), "http");
    }

    // --- encode ---

    #[tokio::test]
    async fn encode_passthrough_small_body() {
        let data = b"hello world".to_vec();
        let result = protocol().encode(data.clone()).await.unwrap();
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn encode_rejects_oversized_body() {
        let big = vec![0u8; 10 * 1024 * 1024 + 1];
        let err = protocol().encode(big).await.unwrap_err();
        assert!(err.to_string().contains("body too large"));
    }

    #[tokio::test]
    async fn encode_accepts_exactly_limit() {
        let exact = vec![0u8; 10 * 1024 * 1024];
        assert!(protocol().encode(exact).await.is_ok());
    }

    #[tokio::test]
    async fn encode_accepts_empty_body() {
        assert_eq!(protocol().encode(vec![]).await.unwrap(), Vec::<u8>::new());
    }

    // --- decode ---

    #[tokio::test]
    async fn decode_passthrough_small_body() {
        let data = b"response body".to_vec();
        let result = protocol().decode(data.clone()).await.unwrap();
        assert_eq!(result, data);
    }

    #[tokio::test]
    async fn decode_rejects_oversized_body() {
        let big = vec![0u8; 10 * 1024 * 1024 + 1];
        let err = protocol().decode(big).await.unwrap_err();
        assert!(err.to_string().contains("body too large"));
    }

    // --- strip_hop_by_hop_headers ---

    #[test]
    fn strips_all_hop_by_hop_headers() {
        let input = headers(&[
            ("connection", "keep-alive"),
            ("keep-alive", "timeout=5"),
            ("proxy-authenticate", "Basic"),
            ("proxy-authorization", "Basic xyz"),
            ("te", "trailers"),
            ("trailers", "Expires"),
            ("transfer-encoding", "chunked"),
            ("upgrade", "websocket"),
            ("content-type", "application/json"),
            ("authorization", "Bearer token"),
        ]);
        let result = strip_hop_by_hop_headers(&input);
        let names: Vec<&str> = result.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["content-type", "authorization"]);
    }

    #[test]
    fn preserves_non_hop_by_hop_headers() {
        let input = headers(&[
            ("content-type", "application/json"),
            ("x-request-id", "abc-123"),
            ("accept", "*/*"),
        ]);
        let result = strip_hop_by_hop_headers(&input);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn strips_case_insensitively() {
        let input = headers(&[
            ("Connection", "close"),
            ("Transfer-Encoding", "chunked"),
            ("Content-Type", "text/plain"),
        ]);
        let result = strip_hop_by_hop_headers(&input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "Content-Type");
    }

    #[test]
    fn empty_headers_returns_empty() {
        assert!(strip_hop_by_hop_headers(&[]).is_empty());
    }
}
