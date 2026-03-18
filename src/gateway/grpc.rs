use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use crate::{
    error::{Error, Result},
    protocols::Protocol,
};
use super::ProtocolGateway;

pub struct GrpcGateway {
    protocol: Arc<dyn Protocol>,
}

impl GrpcGateway {
    pub fn new(protocol: Arc<dyn Protocol>) -> Self {
        Self { protocol }
    }

    /// Proxy a gRPC request to an upstream gRPC server over HTTP/2.
    ///
    /// - `target_base`: base URL of the upstream gRPC server (e.g. `"http://service:50051"`)
    /// - `path`: full gRPC method path (e.g. `"/helloworld.Greeter/SayHello"`)
    /// - `headers`: sanitized forwarded headers
    /// - `body`: gRPC-framed protobuf bytes (5-byte header + payload)
    /// - `timeout_secs`: per-request timeout
    ///
    /// Sets the required gRPC headers (`content-type: application/grpc`,
    /// `te: trailers`) and forwards the framed payload over HTTP/2 POST.
    /// Returns `(status_code, response_headers, response_body)`.
    ///
    /// SECURITY: `target_base` must always come from `RouteConfig.target` (config-defined),
    /// never from request data, preventing SSRF.
    pub async fn proxy(
        &self,
        target_base: &str,
        path: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
        timeout_secs: u64,
    ) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
        if !target_base.starts_with("http://") && !target_base.starts_with("https://") {
            return Err(Error::Protocol(
                "gRPC target URL must start with http:// or https://".to_string(),
            ));
        }

        // Path traversal guard.
        if crate::protocols::http::path_contains_traversal(path) {
            return Err(Error::Protocol("invalid path".to_string()));
        }

        let url = format!("{}{}", target_base.trim_end_matches('/'), path);

        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(timeout_secs))
            .http2_prior_knowledge() // gRPC requires HTTP/2
            .build()
            .map_err(|e| Error::Protocol(e.to_string()))?;

        // Strip hop-by-hop and Host; we set gRPC-required headers explicitly.
        let sanitized = crate::protocols::http::strip_hop_by_hop_headers(headers);
        let forward_headers: Vec<(String, String)> = sanitized
            .into_iter()
            .filter(|(name, _)| {
                let lower = name.to_lowercase();
                lower != "host"
                    && lower != "content-type"
                    && lower != "content-length"
                    && lower != "te"
            })
            .collect();

        let mut req = client
            .post(&url)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .header("x-forwarded-for", "gateway")
            .body(body);

        for (name, value) in &forward_headers {
            req = req.header(name.as_str(), value.as_str());
        }

        let response = req.send().await.map_err(|e| Error::Protocol(e.to_string()))?;
        let status = response.status().as_u16();

        let raw_resp_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
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
impl ProtocolGateway for GrpcGateway {
    async fn handle_request(&self, _request: Vec<u8>) -> Result<Vec<u8>> {
        // Raw byte handle_request not used — callers use proxy() directly.
        Ok(Vec::new())
    }

    fn protocol(&self) -> Arc<dyn Protocol> {
        self.protocol.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::grpc::GrpcProtocol;

    fn gateway() -> GrpcGateway {
        let protocol = Arc::new(GrpcProtocol::new(serde_json::Value::Null).unwrap());
        GrpcGateway::new(protocol)
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let gw = gateway();
        let err = gw
            .proxy("file:///etc/passwd", "/pkg.Svc/Method", &[], vec![], 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("http://") || err.to_string().contains("https://"));
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let gw = gateway();
        let err = gw
            .proxy("http://127.0.0.1:50051", "/../etc/passwd", &[], vec![], 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid path"));
    }

    #[tokio::test]
    async fn accepts_valid_grpc_path() {
        let gw = gateway();
        // Should pass scheme/path validation; any failure will be a network error.
        let result = gw
            .proxy(
                "http://127.0.0.1:19998",
                "/helloworld.Greeter/SayHello",
                &[],
                vec![],
                1,
            )
            .await;
        if let Err(e) = result {
            assert!(
                !e.to_string().contains("must start with http"),
                "unexpected scheme error: {e}"
            );
            assert!(
                !e.to_string().contains("invalid path"),
                "unexpected path error: {e}"
            );
        }
    }
}
