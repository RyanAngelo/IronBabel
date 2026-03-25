use async_trait::async_trait;
use serde_json::Value;

use crate::error::{Error, Result};
use super::Protocol;

const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

pub struct AmqpProtocol {
    name: String,
}

impl AmqpProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "amqp".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for AmqpProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(Error::Protocol("AMQP message too large".to_string()));
        }
        Ok(data)
    }

    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(Error::Protocol("AMQP message too large".to_string()));
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> AmqpProtocol {
        AmqpProtocol::new(serde_json::Value::Null).unwrap()
    }

    #[test]
    fn name_is_amqp() {
        assert_eq!(protocol().name(), "amqp");
    }

    #[tokio::test]
    async fn encode_passthrough() {
        let data = b"hello amqp".to_vec();
        assert_eq!(protocol().encode(data.clone()).await.unwrap(), data);
    }

    #[tokio::test]
    async fn decode_passthrough() {
        let data = b"reply payload".to_vec();
        assert_eq!(protocol().decode(data.clone()).await.unwrap(), data);
    }

    #[tokio::test]
    async fn encode_rejects_oversized() {
        let big = vec![0u8; MAX_MESSAGE_SIZE + 1];
        assert!(protocol().encode(big).await.is_err());
    }

    #[tokio::test]
    async fn decode_rejects_oversized() {
        let big = vec![0u8; MAX_MESSAGE_SIZE + 1];
        assert!(protocol().decode(big).await.is_err());
    }
}
