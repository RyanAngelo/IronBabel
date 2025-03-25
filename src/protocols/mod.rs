use async_trait::async_trait;
use std::sync::Arc;

use crate::error::Result;

pub mod http;
pub mod grpc;
pub mod graphql;
pub mod mqtt;
pub mod ws;

#[async_trait]
pub trait Protocol: Send + Sync {
    fn name(&self) -> &str;
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
} 