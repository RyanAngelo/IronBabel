use async_trait::async_trait;
use serde_json::Value;
use crate::error::Result;
use super::Protocol;

pub struct GrpcProtocol {
    name: String,
}

impl GrpcProtocol {
    pub fn new(settings: Value) -> Result<Self> {
        Ok(Self {
            name: "grpc".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for GrpcProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement gRPC encoding
        Ok(data)
    }

    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement gRPC decoding
        Ok(data)
    }
} 