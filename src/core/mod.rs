use async_trait::async_trait;
use std::sync::Arc;

use crate::error::Result;
use crate::protocols::Protocol;

pub mod gateway;
pub mod router;

#[async_trait]
pub trait Gateway: Send + Sync {
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    fn protocols(&self) -> Vec<Arc<dyn Protocol>>;
} 