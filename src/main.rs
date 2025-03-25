use iron_babel::config::GatewayConfig;
use iron_babel::core::Gateway;
use iron_babel::error::Result;
use iron_babel::core::gateway;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    iron_babel::utils::logging::init()?;
    
    // Load configuration
    let config = GatewayConfig::load().await?;
    println!("Config: {:?}", config);
    
    // Create and start gateway
    let gateway_config = gateway::GatewayConfig::from_config(config)?;
    let gateway = gateway::create_gateway(gateway_config)?;
    gateway.start().await?;
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    
    // Graceful shutdown
    gateway.stop().await?;
    
    Ok(())
} 