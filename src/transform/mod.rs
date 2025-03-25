use async_trait::async_trait;

use crate::error::Result;

pub mod json;
pub mod protobuf;

#[async_trait]
pub trait Transformer: Send + Sync {
    async fn transform(&self, input: Vec<u8>, from_format: &str, to_format: &str) -> Result<Vec<u8>>;
} 