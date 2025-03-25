use std::sync::Arc;
use async_trait::async_trait;
use crate::{error::Result, protocols::Protocol};
use crate::protocols::{http::HttpProtocol, grpc::GrpcProtocol, graphql::GraphQLProtocol, mqtt::MqttProtocol, ws::WebSocketProtocol};
use super::Gateway;

pub struct GatewayConfig {
    pub protocols: Vec<Arc<dyn Protocol>>,
}

impl GatewayConfig {
    pub fn from_config(config: crate::config::GatewayConfig) -> Result<Self> {
        let mut protocols = Vec::new();
        
        for protocol_config in config.protocols {
            let protocol: Arc<dyn Protocol> = match protocol_config.name.as_str() {
                "http" => Arc::new(HttpProtocol::new(protocol_config.settings)?),
                "grpc" => Arc::new(GrpcProtocol::new(protocol_config.settings)?),
                "graphql" => Arc::new(GraphQLProtocol::new(protocol_config.settings)?),
                "mqtt" => Arc::new(MqttProtocol::new(protocol_config.settings)?),
                "websocket" => Arc::new(WebSocketProtocol::new(protocol_config.settings)?),
                _ => return Err(crate::error::Error::Protocol(format!("Unsupported protocol: {}", protocol_config.name))),
            };
            
            if protocol_config.enabled {
                protocols.push(protocol);
            }
        }
        
        Ok(Self { protocols })
    }
}

pub struct DefaultGateway {
    protocols: Vec<Arc<dyn Protocol>>,
}

#[async_trait]
impl Gateway for DefaultGateway {
    async fn start(&self) -> Result<()> {
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    fn protocols(&self) -> Vec<Arc<dyn Protocol>> {
        self.protocols.clone()
    }
}

pub fn create_gateway(config: GatewayConfig) -> Result<DefaultGateway> {
    Ok(DefaultGateway {
        protocols: config.protocols,
    })
} 