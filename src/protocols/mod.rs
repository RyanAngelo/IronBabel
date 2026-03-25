use async_trait::async_trait;

use crate::error::Result;

pub mod grpc;
pub mod graphql;
pub mod amqp;
pub mod http;
pub mod mqtt;
pub mod ws;
pub mod zmq;

#[async_trait]
pub trait Protocol: Send + Sync {
    fn name(&self) -> &str;
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>>;
} 
