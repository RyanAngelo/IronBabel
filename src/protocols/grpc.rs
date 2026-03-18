use async_trait::async_trait;
use serde_json::Value;
use crate::error::{Error, Result};
use super::Protocol;

/// Minimum gRPC frame header size: 1-byte compression flag + 4-byte length.
const GRPC_HEADER_LEN: usize = 5;

pub struct GrpcProtocol {
    name: String,
}

impl GrpcProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "grpc".to_string(),
        })
    }
}

/// Wrap raw protobuf bytes in a gRPC length-prefixed frame.
///
/// gRPC wire format (per https://grpc.io/docs/what-is-grpc/core-concepts/):
/// ```text
/// [ compression-flag: u8 (0 = uncompressed) ]
/// [ message-length:   u32 big-endian        ]
/// [ message-payload:  bytes ...              ]
/// ```
pub fn encode_grpc_frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(GRPC_HEADER_LEN + payload.len());
    out.push(0u8); // no compression
    let len = payload.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Strip the gRPC length-prefixed frame header and return the raw payload.
///
/// Returns an error if the data is too short, the compression flag indicates
/// an unsupported compression algorithm, or the declared length does not match
/// the actual payload length.
pub fn decode_grpc_frame(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < GRPC_HEADER_LEN {
        return Err(Error::Protocol(format!(
            "gRPC frame too short: {} bytes (expected at least {})",
            data.len(),
            GRPC_HEADER_LEN,
        )));
    }

    let compression_flag = data[0];
    if compression_flag != 0 {
        return Err(Error::Protocol(format!(
            "Unsupported gRPC compression flag: {} (only uncompressed (0) is supported)",
            compression_flag,
        )));
    }

    let declared_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
    let payload = &data[GRPC_HEADER_LEN..];

    if payload.len() < declared_len {
        return Err(Error::Protocol(format!(
            "gRPC frame payload truncated: declared {} bytes, got {}",
            declared_len,
            payload.len(),
        )));
    }

    Ok(payload[..declared_len].to_vec())
}

#[async_trait]
impl Protocol for GrpcProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    /// Wrap the raw protobuf payload in a gRPC length-prefixed frame.
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        Ok(encode_grpc_frame(&data))
    }

    /// Strip the gRPC length-prefixed frame header, returning the raw protobuf bytes.
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        decode_grpc_frame(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proto() -> GrpcProtocol {
        GrpcProtocol::new(serde_json::Value::Null).unwrap()
    }

    #[tokio::test]
    async fn encode_adds_grpc_header() {
        let p = proto();
        let payload = b"hello".to_vec();
        let framed = p.encode(payload.clone()).await.unwrap();

        assert_eq!(framed.len(), GRPC_HEADER_LEN + payload.len());
        assert_eq!(framed[0], 0); // no compression
        let len = u32::from_be_bytes([framed[1], framed[2], framed[3], framed[4]]);
        assert_eq!(len as usize, payload.len());
        assert_eq!(&framed[GRPC_HEADER_LEN..], payload.as_slice());
    }

    #[tokio::test]
    async fn decode_strips_grpc_header() {
        let p = proto();
        let payload = b"world".to_vec();
        let framed = encode_grpc_frame(&payload);
        let decoded = p.decode(framed).await.unwrap();
        assert_eq!(decoded, payload);
    }

    #[tokio::test]
    async fn round_trip() {
        let p = proto();
        let original = b"\x0a\x05hello".to_vec(); // minimal protobuf field 1 = "hello"
        let encoded = p.encode(original.clone()).await.unwrap();
        let decoded = p.decode(encoded).await.unwrap();
        assert_eq!(decoded, original);
    }

    #[tokio::test]
    async fn decode_rejects_too_short() {
        let p = proto();
        let err = p.decode(vec![0x00, 0x00]).await.unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[tokio::test]
    async fn decode_rejects_compressed() {
        let p = proto();
        let mut framed = encode_grpc_frame(b"data");
        framed[0] = 1; // set compression flag
        let err = p.decode(framed).await.unwrap_err();
        assert!(err.to_string().contains("compression"));
    }

    #[tokio::test]
    async fn decode_rejects_truncated_payload() {
        let p = proto();
        // Claim payload is 100 bytes but provide only 3
        let mut framed = vec![0u8; GRPC_HEADER_LEN + 3];
        framed[0] = 0;
        let declared: u32 = 100;
        framed[1..5].copy_from_slice(&declared.to_be_bytes());
        let err = p.decode(framed).await.unwrap_err();
        assert!(err.to_string().contains("truncated"));
    }

    #[test]
    fn encode_grpc_frame_empty_payload() {
        let framed = encode_grpc_frame(&[]);
        assert_eq!(framed.len(), GRPC_HEADER_LEN);
        assert_eq!(framed[0], 0);
        let len = u32::from_be_bytes([framed[1], framed[2], framed[3], framed[4]]);
        assert_eq!(len, 0);
    }

    #[test]
    fn decode_grpc_frame_exact_payload() {
        let payload = vec![1u8, 2, 3, 4, 5];
        let framed = encode_grpc_frame(&payload);
        let decoded = decode_grpc_frame(&framed).unwrap();
        assert_eq!(decoded, payload);
    }
}
