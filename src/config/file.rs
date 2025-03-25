use crate::config::GatewayConfig;
use crate::error::Result;
use serde_yaml;

/// Loads configuration from a YAML file
pub async fn load_from_file(path: String) -> Result<GatewayConfig> {
    let contents = tokio::fs::read_to_string(path).await?;
    let config: GatewayConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
} 