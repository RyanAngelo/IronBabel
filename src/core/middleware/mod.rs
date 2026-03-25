mod auth;
mod rate_limit;
mod logging;

pub use auth::AuthMiddleware;
pub use rate_limit::RateLimitMiddleware;
pub use logging::LoggingMiddleware;

use std::sync::Arc;
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::Result;

#[async_trait::async_trait]
pub trait Middleware: Send + Sync {
    async fn handle_request(&self, request: Request) -> Result<Request>;
    async fn handle_response(&self, response: Response) -> Result<Response>;
    fn config(&self) -> &MiddlewareConfig;
}

#[derive(Clone)]
pub struct MiddlewareChain {
    middlewares: Vec<Arc<dyn Middleware>>,
}

impl MiddlewareChain {
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    pub fn add(&mut self, middleware: Arc<dyn Middleware>) {
        self.middlewares.push(middleware);
    }

    pub async fn handle_request(&self, mut request: Request) -> Result<Request> {
        for middleware in &self.middlewares {
            request = middleware.handle_request(request).await?;
        }
        Ok(request)
    }

    pub async fn handle_response(&self, mut response: Response) -> Result<Response> {
        for middleware in self.middlewares.iter().rev() {
            response = middleware.handle_response(response).await?;
        }
        Ok(response)
    }

    /// Returns the configured rate-limit window in seconds, if any rate-limit
    /// middleware is present and enabled. Used to populate the `Retry-After`
    /// response header on 429 responses.
    pub fn rate_limit_window_secs(&self) -> Option<u64> {
        self.middlewares.iter().find_map(|m| {
            let cfg = m.config();
            if !cfg.enabled {
                return None;
            }
            cfg.settings
                .get("window_secs")
                .and_then(|v| v.as_u64())
        })
    }
}

impl Default for MiddlewareChain {
    fn default() -> Self {
        Self::new()
    }
}
