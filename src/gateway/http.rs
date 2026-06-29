use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use crate::{error::{Error, Result}, protocols::Protocol};
use super::ProtocolGateway;

pub struct HttpGateway {
    protocol: Arc<dyn Protocol>,
    /// Shared HTTP client. Built once so the connection pool and TLS config are
    /// reused across requests; per-request timeouts are applied per call.
    client: reqwest::Client,
}

impl HttpGateway {
    pub fn new(protocol: Arc<dyn Protocol>) -> Self {
        Self {
            protocol,
            client: reqwest::Client::new(),
        }
    }

    /// Proxy an incoming request to a backend target.
    ///
    /// - `method`: HTTP method to use
    /// - `target_base`: base URL from RouteConfig (e.g. "http://127.0.0.1:9000")
    /// - `path`: path portion of the incoming request
    /// - `query`: optional query string (without leading `?`)
    /// - `headers`: sanitized request headers as (name, value) pairs
    /// - `body`: raw request body bytes
    /// - `timeout_secs`: per-request timeout in seconds
    ///
    /// Returns `(status_code, response_headers, response_body)`.
    ///
    /// SECURITY: `target_base` must always come from `RouteConfig.target` (config-defined),
    /// never from request data, preventing SSRF.
    pub async fn proxy(
        &self,
        method: reqwest::Method,
        target_base: &str,
        path: &str,
        query: Option<&str>,
        headers: &[(String, String)],
        body: Vec<u8>,
        timeout_secs: u64,
    ) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
        // Validate target scheme (only http/https allowed)
        if !target_base.starts_with("http://") && !target_base.starts_with("https://") {
            return Err(Error::Protocol(
                "target_base must start with http:// or https://".to_string(),
            ));
        }

        // Path traversal guard: reject raw and percent-encoded ".." segments.
        if crate::protocols::http::path_contains_traversal(path) {
            return Err(Error::Protocol("invalid path".to_string()));
        }

        // SSRF guard: the path is concatenated onto the trusted base URL, so it
        // must be a well-formed absolute path with no characters that could
        // alter the resolved authority (e.g. `@evil.com`) or inject into the
        // request line.
        if !crate::protocols::http::path_is_safe(path) {
            return Err(Error::Protocol("invalid path".to_string()));
        }
        if let Some(q) = query {
            if !crate::protocols::http::query_is_safe(q) {
                return Err(Error::Protocol("invalid query".to_string()));
            }
        }

        // Build target URL
        let url = if let Some(q) = query {
            format!("{}{}?{}", target_base.trim_end_matches('/'), path, q)
        } else {
            format!("{}{}", target_base.trim_end_matches('/'), path)
        };

        // Strip hop-by-hop headers and drop Host (let reqwest set it to the target host)
        let sanitized = crate::protocols::http::strip_hop_by_hop_headers(headers);
        let forward_headers: Vec<(String, String)> = sanitized
            .into_iter()
            .filter(|(name, _)| name.to_lowercase() != "host")
            .collect();

        // Assemble and send request, applying the per-request timeout to the
        // shared client.
        let mut req = self
            .client
            .request(method, &url)
            .timeout(Duration::from_secs(timeout_secs))
            .body(body);
        for (name, value) in &forward_headers {
            req = req.header(name.as_str(), value.as_str());
        }

        let response = req.send().await.map_err(|e| Error::Protocol(e.to_string()))?;

        let status = response.status().as_u16();

        // Collect and sanitize response headers. Headers whose value is not
        // valid UTF-8 are skipped (rather than forwarded as an empty value).
        let raw_resp_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str().ok().map(|val| (k.as_str().to_string(), val.to_string()))
            })
            .collect();
        let resp_headers = crate::protocols::http::strip_hop_by_hop_headers(&raw_resp_headers);

        let resp_body = response
            .bytes()
            .await
            .map_err(|e| Error::Protocol(e.to_string()))?
            .to_vec();

        Ok((status, resp_headers, resp_body))
    }
}

#[async_trait]
impl ProtocolGateway for HttpGateway {
    async fn handle_request(&self, _request: Vec<u8>) -> Result<Vec<u8>> {
        // Raw byte handle_request is not used for HTTP — callers use proxy() directly.
        Ok(Vec::new())
    }

    fn protocol(&self) -> Arc<dyn Protocol> {
        self.protocol.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::protocols::http::HttpProtocol;

    fn gateway() -> HttpGateway {
        let protocol = Arc::new(HttpProtocol::new(serde_json::Value::Null).unwrap());
        HttpGateway::new(protocol)
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "http://127.0.0.1:9000",
                "/../etc/passwd",
                None,
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid path"));
    }

    #[tokio::test]
    async fn rejects_path_traversal_middle_segment() {
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "http://127.0.0.1:9000",
                "/api/../secrets",
                None,
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid path"));
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "file:///etc/passwd",
                "/",
                None,
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("http://") || err.to_string().contains("https://"));
    }

    #[tokio::test]
    async fn rejects_ftp_scheme() {
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "ftp://internal-server/data",
                "/",
                None,
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("http://") || err.to_string().contains("https://"));
    }

    #[tokio::test]
    async fn rejects_authority_confusion_path() {
        // A path that does not start with '/' could turn the trusted base host
        // into userinfo (SSRF). It must be rejected before any request is sent.
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "http://127.0.0.1:9000",
                "@evil.com/",
                None,
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid path"));
    }

    #[tokio::test]
    async fn rejects_query_with_control_chars() {
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "http://127.0.0.1:9000",
                "/api",
                Some("a=1\r\nHost: evil"),
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid query"));
    }

    #[tokio::test]
    async fn rejects_percent_encoded_path_traversal() {
        let gw = gateway();
        let err = gw
            .proxy(
                reqwest::Method::GET,
                "http://127.0.0.1:9000",
                "/%2e%2e/etc/passwd",
                None,
                &[],
                vec![],
                5,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid path"));
    }

    #[tokio::test]
    async fn accepts_https_scheme() {
        // https:// should pass scheme validation; any failure will be a network error.
        let gw = gateway();
        let result = gw
            .proxy(
                reqwest::Method::GET,
                "https://127.0.0.1:19999",
                "/",
                None,
                &[],
                vec![],
                1,
            )
            .await;
        // Should get a network/timeout error, NOT our scheme validation message.
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("must start with http"),
                "got unexpected scheme validation error: {e}"
            );
        }
    }
}
