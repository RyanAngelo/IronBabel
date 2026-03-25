use async_trait::async_trait;
use tracing::info;
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
        if !self.config.enabled {
            return Ok(request);
        }

        info!(
            method = request.metadata.method.as_deref().unwrap_or("UNKNOWN"),
            path = request.metadata.path.as_deref().unwrap_or("/"),
            "Incoming request"
        );
        Ok(request)
    }

    async fn handle_response(&self, response: Response) -> Result<Response> {
        if !self.config.enabled {
            return Ok(response);
        }

        info!(
            status = response.metadata.status_code.unwrap_or(0),
            "Outgoing response"
        );
        Ok(response)
    }

    fn config(&self) -> &MiddlewareConfig {
        &self.config
    }
}
