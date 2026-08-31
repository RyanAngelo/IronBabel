use async_trait::async_trait;
use std::sync::Arc;

use crate::error::Result;
use crate::protocols::Protocol;

pub mod gateway;
pub mod middleware;
pub mod router;
pub mod shutdown;
pub mod types;

pub use middleware::{MiddlewareChain, AuthMiddleware, RateLimitMiddleware, LoggingMiddleware};
pub use router::Router;
pub use shutdown::Shutdown;
pub use types::{MiddlewareConfig, Request, RequestMetadata, Response, ResponseMetadata};

#[async_trait]
pub trait Gateway: Send + Sync {
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    fn protocols(&self) -> Vec<Arc<dyn Protocol>>;
}
