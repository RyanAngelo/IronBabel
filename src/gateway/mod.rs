use async_trait::async_trait;
use std::sync::Arc;

use crate::error::Result;
use crate::protocols::Protocol;

pub mod grpc;
pub mod graphql;
pub mod http;
pub mod zmq;

#[async_trait]
pub trait ProtocolGateway: Send + Sync {
    async fn handle_request(&self, request: Vec<u8>) -> Result<Vec<u8>>;
    fn protocol(&self) -> Arc<dyn Protocol>;
} 