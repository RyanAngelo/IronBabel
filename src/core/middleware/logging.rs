use async_trait::async_trait;
use tracing::{info, error};
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::Result;

pub struct LoggingMiddleware {
    config: MiddlewareConfig,
}

impl LoggingMiddleware {
    pub fn new(config: MiddlewareConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl super::Middleware for LoggingMiddleware {
    async fn handle_request(&self, request: Request) -> Result<Request> {
        info!(
            "Incoming request: {} {}",
            request.method(),
            request.uri()
        );
        Ok(request)
    }

    async fn handle_response(&self, response: Response) -> Result<Response> {
        info!(
            "Outgoing response: {} {}",
            response.status(),
            response.status().canonical_reason().unwrap_or("")
        );
        Ok(response)
    }

    fn config(&self) -> &MiddlewareConfig {
        &self.config
    }
} 