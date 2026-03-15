use async_trait::async_trait;
use serde_json::Value;
use crate::error::Result;
use super::Protocol;

const MAX_FRAME_SIZE: usize = 10 * 1024 * 1024; // 10 MB

pub struct ZmqProtocol {
    name: String,
}

impl ZmqProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "zmq".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for ZmqProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    /// Encode validates size and passes bytes through unchanged.
    /// Actual ZMQ framing is handled by ZmqGateway.
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() > MAX_FRAME_SIZE {
            return Err(crate::error::Error::Protocol("ZMQ frame too large".to_string()));
        }
        Ok(data)
    }

    /// Decode validates size and passes bytes through unchanged.
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() > MAX_FRAME_SIZE {
            return Err(crate::error::Error::Protocol("ZMQ frame too large".to_string()));
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> ZmqProtocol {
        ZmqProtocol::new(serde_json::Value::Null).unwrap()
    }

    #[test]
    fn name_is_zmq() {
        assert_eq!(protocol().name(), "zmq");
    }

    #[tokio::test]
    async fn encode_passthrough() {
        let data = b"hello zmq".to_vec();
        assert_eq!(protocol().encode(data.clone()).await.unwrap(), data);
    }

    #[tokio::test]
    async fn decode_passthrough() {
        let data = b"reply frame".to_vec();
        assert_eq!(protocol().decode(data.clone()).await.unwrap(), data);
    }

    #[tokio::test]
    async fn encode_rejects_oversized() {
        let big = vec![0u8; MAX_FRAME_SIZE + 1];
        assert!(protocol().encode(big).await.is_err());
    }

    #[tokio::test]
    async fn decode_rejects_oversized() {
        let big = vec![0u8; MAX_FRAME_SIZE + 1];
        assert!(protocol().decode(big).await.is_err());
    }
}
