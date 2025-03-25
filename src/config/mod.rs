use serde::{Deserialize, Serialize};
use crate::error::Result;
use crate::config::file::load_from_file;

#[derive(Debug, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub host: String,
    pub protocols: Vec<ProtocolConfig>,
}

impl GatewayConfig {
    /// Loads configuration from the default path
    pub async fn load() -> Result<Self> {
        load_from_file("config/gateway.yaml".to_string()).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProtocolConfig {
    pub name: String,
    pub enabled: bool,
    pub settings: serde_json::Value,
}

pub mod file;
pub mod env; 