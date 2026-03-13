use iron_babel::config::GatewayConfig;
use iron_babel::core::Gateway;
use iron_babel::error::Result;
use iron_babel::core::gateway;

const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

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
    tracing::info!("Shutdown signal received; stopping gateway...");

    // Graceful shutdown with timeout — avoids hanging forever if tasks stall.
    match tokio::time::timeout(
        std::time::Duration::from_secs(SHUTDOWN_TIMEOUT_SECS),
        gateway.stop(),
    )
    .await
    {
        Ok(Ok(())) => tracing::info!("Gateway stopped cleanly."),
        Ok(Err(e)) => tracing::error!("Error during shutdown: {}", e),
        Err(_) => tracing::warn!(
            "Gateway did not stop within {}s; forcing exit.",
            SHUTDOWN_TIMEOUT_SECS
        ),
    }

    Ok(())
} 