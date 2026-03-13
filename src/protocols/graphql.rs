use async_trait::async_trait;
use serde_json::Value;
use crate::error::Result;
use super::Protocol;

pub struct GraphQLProtocol {
    name: String,
}

impl GraphQLProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "graphql".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for GraphQLProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement GraphQL encoding
        Ok(data)
    }

    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement GraphQL decoding
        Ok(data)
    }
} 