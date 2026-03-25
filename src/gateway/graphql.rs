use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use crate::{
    error::{Error, Result},
    protocols::Protocol,
};
use super::ProtocolGateway;

pub struct GraphQLGateway {
    protocol: Arc<dyn Protocol>,
}

impl GraphQLGateway {
    pub fn new(protocol: Arc<dyn Protocol>) -> Self {
        Self { protocol }
    }

    /// Forward a GraphQL request to `target_url` as an HTTP POST.
    ///
    /// - `target_url`: full URL of the GraphQL endpoint (e.g. `"http://api:4000/graphql"`)
    /// - `headers`: sanitized forwarded headers
    /// - `body`: validated GraphQL JSON bytes (already checked by `GraphQLProtocol::encode`)
    /// - `timeout_secs`: per-request timeout
    ///
    /// Always POSTs with `Content-Type: application/json` as required by the
    /// GraphQL-over-HTTP spec. Returns `(status_code, response_headers, response_body)`.
    pub async fn proxy(
        &self,
        target_url: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
        timeout_secs: u64,
    ) -> Result<(u16, Vec<(String, String)>, Vec<u8>)> {
        if !target_url.starts_with("http://") && !target_url.starts_with("https://") {
            return Err(Error::GraphQL(
                "GraphQL target URL must start with http:// or https://".to_string(),
            ));
        }

        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| Error::GraphQL(e.to_string()))?;

        // Strip hop-by-hop and Host headers; GraphQL always uses POST + JSON.
        let sanitized = crate::protocols::http::strip_hop_by_hop_headers(headers);
        let forward_headers: Vec<(String, String)> = sanitized
            .into_iter()
            .filter(|(name, _)| {
                let lower = name.to_lowercase();
                lower != "host" && lower != "content-type" && lower != "content-length"
            })
            .collect();

        let mut req = client
            .post(target_url)
            .header("content-type", "application/json")
            .body(body);

        for (name, value) in &forward_headers {
            req = req.header(name.as_str(), value.as_str());
        }

        let response = req.send().await.map_err(|e| Error::GraphQL(e.to_string()))?;
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
            .map_err(|e| Error::GraphQL(e.to_string()))?
            .to_vec();

        Ok((status, resp_headers, resp_body))
    }
}

#[async_trait]
impl ProtocolGateway for GraphQLGateway {
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
    use crate::protocols::graphql::GraphQLProtocol;

    fn gateway() -> GraphQLGateway {
        let protocol = Arc::new(GraphQLProtocol::new(serde_json::Value::Null).unwrap());
        GraphQLGateway::new(protocol)
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let gw = gateway();
        let err = gw
            .proxy("file:///etc/passwd", &[], b"{}".to_vec(), 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("http://") || err.to_string().contains("https://"));
    }

    #[tokio::test]
    async fn rejects_ftp_scheme() {
        let gw = gateway();
        let err = gw
            .proxy("ftp://internal/graphql", &[], b"{}".to_vec(), 5)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("http://") || err.to_string().contains("https://"));
    }
}
