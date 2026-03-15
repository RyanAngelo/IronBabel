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

/// Returns `true` if `path` contains a `..` traversal segment in raw or
/// percent-encoded form (e.g. `%2e%2e`, `.%2e`, `%2e.`).
///
/// The check iteratively decodes percent-encoded dots so that multi-level
/// encoding such as `%252e%252e` is also rejected.
pub fn path_contains_traversal(path: &str) -> bool {
    let mut p = path.to_ascii_lowercase();
    // Iteratively decode percent-encoded percent signs (%25 → %) and dots
    // (%2e → .) to catch multi-level encoding such as %252e%252e → %2e%2e → ..
    loop {
        let decoded = p.replace("%25", "%").replace("%2e", ".");
        if decoded == p {
            break;
        }
        p = decoded;
    }
    p.split('/').any(|seg| seg == "..")
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

    // --- path_contains_traversal ---

    #[test]
    fn detects_plain_traversal() {
        assert!(path_contains_traversal("/../etc/passwd"));
        assert!(path_contains_traversal("/api/../secret"));
    }

    #[test]
    fn detects_percent_encoded_traversal() {
        assert!(path_contains_traversal("/%2e%2e/etc/passwd"));
        assert!(path_contains_traversal("/%2E%2E/etc/passwd"));
        assert!(path_contains_traversal("/.%2e/etc"));
        assert!(path_contains_traversal("/%2e./etc"));
    }

    #[test]
    fn detects_double_encoded_traversal() {
        assert!(path_contains_traversal("/%252e%252e/etc"));
    }

    #[test]
    fn allows_normal_paths() {
        assert!(!path_contains_traversal("/api/users"));
        assert!(!path_contains_traversal("/"));
        assert!(!path_contains_traversal("/api/v2"));
        assert!(!path_contains_traversal("/foo.bar/baz"));
    }
}
