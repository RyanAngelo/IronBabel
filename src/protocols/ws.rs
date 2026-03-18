use async_trait::async_trait;
use serde_json::Value;
use crate::error::Result;
use super::Protocol;

pub struct WebSocketProtocol {
    name: String,
}

impl WebSocketProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "websocket".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for WebSocketProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    /// WebSocket frames are passed through as raw bytes.
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        Ok(data)
    }

    /// WebSocket frames are passed through as raw bytes.
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        Ok(data)
    }
}
