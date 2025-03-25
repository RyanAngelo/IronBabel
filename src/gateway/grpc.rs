use async_trait::async_trait;
use std::sync::Arc;
use crate::{error::Result, protocols::Protocol};
use super::ProtocolGateway;

pub struct GrpcGateway {
    protocol: Arc<dyn Protocol>,
}

#[async_trait]
impl ProtocolGateway for GrpcGateway {
    async fn handle_request(&self, _request: Vec<u8>) -> Result<Vec<u8>> {
        // TODO: Implement gRPC request handling
        Ok(Vec::new())
    }

    fn protocol(&self) -> Arc<dyn Protocol> {
        self.protocol.clone()
    }
} 