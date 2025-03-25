mod common;

use iron_babel::{
    core::Gateway,
    error::Result,
    gateway::ProtocolGateway,
    protocols::Protocol,
};
use std::sync::Arc;
use self::common::{create_test_data, MockGateway, MockProtocol};

#[tokio::test]
async fn test_gateway_lifecycle() -> Result<()> {
    let protocol = Arc::new(MockProtocol::new("test_protocol"));
    let gateway = MockGateway::new(protocol.clone());
    
    // Test protocol name
    assert_eq!(gateway.protocol().name(), "test_protocol");
    
    // Test request handling
    let test_data = create_test_data(100);
    let response = gateway.handle_request(test_data.clone()).await?;
    assert_eq!(response, test_data);
    
    Ok(())
}

#[tokio::test]
async fn test_protocol_encoding_decoding() -> Result<()> {
    let protocol = Arc::new(MockProtocol::new("test_protocol"));
    let test_data = create_test_data(100);
    
    // Test encoding
    let encoded = protocol.encode(test_data.clone()).await?;
    assert_eq!(encoded, test_data);
    
    // Test decoding
    let decoded = protocol.decode(test_data.clone()).await?;
    assert_eq!(decoded, test_data);
    
    // Verify calls were recorded
    let encode_calls = protocol.get_encode_calls().await;
    let decode_calls = protocol.get_decode_calls().await;
    
    assert_eq!(encode_calls.len(), 1);
    assert_eq!(decode_calls.len(), 1);
    assert_eq!(encode_calls[0], test_data);
    assert_eq!(decode_calls[0], test_data);
    
    Ok(())
} 