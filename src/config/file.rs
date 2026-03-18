use crate::config::GatewayConfig;
use crate::error::{Error, Result};

/// Loads and validates configuration from a YAML file.
///
/// Returns a descriptive error that includes the file path on failure.
pub async fn load_from_file(path: String) -> Result<GatewayConfig> {
    let contents = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| Error::Config(format!("Could not read config file '{}': {}", path, e)))?;

    let config: GatewayConfig = serde_yaml::from_str(&contents)
        .map_err(|e| Error::Config(format!("Invalid YAML in config file '{}': {}", path, e)))?;

    config.validate()?;

    Ok(config)
}
