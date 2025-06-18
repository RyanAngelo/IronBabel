use async_trait::async_trait;
use serde_json::Value;
use crate::error::Result;
use super::Protocol;

pub struct HttpProtocol {
    name: String,
}

impl HttpProtocol {
    pub fn new(settings: Value) -> Result<Self> {
        Ok(Self {
            name: "http".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for HttpProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement HTTP encoding
        Ok(data)
    }

    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement HTTP decoding
        Ok(data)
    }
} 