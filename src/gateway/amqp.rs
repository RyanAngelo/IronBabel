use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use lapin::{
    BasicProperties, Connection, ConnectionProperties, options::{
        BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions,
    }, types::FieldTable,
};
use tokio::sync::Notify;

use crate::config::{AmqpConsumeListenerConfig, AmqpTransportConfig};
use crate::error::{Error, Result};

static CONSUMER_TAG_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct AmqpGateway;

impl AmqpGateway {
    pub fn new() -> Self {
        Self
    }

    pub async fn publish(&self, cfg: &AmqpTransportConfig, body: Vec<u8>) -> Result<()> {
        let publish = async {
            let connection = Connection::connect(&cfg.broker_url, ConnectionProperties::default())
                .await
                .map_err(|e| Error::Protocol(format!("AMQP connect failed: {}", e)))?;
            let channel = connection
                .create_channel()
                .await
                .map_err(|e| Error::Protocol(format!("AMQP create channel failed: {}", e)))?;

            let mut properties = BasicProperties::default();
            if cfg.persistent {
                properties = properties.with_delivery_mode(2);
            }
            if let Some(content_type) = &cfg.content_type {
                properties = properties.with_content_type(content_type.clone().into());
            }

            channel
                .basic_publish(
                    &cfg.exchange,
                    &cfg.routing_key,
                    BasicPublishOptions {
                        mandatory: cfg.mandatory,
                        ..Default::default()
                    },
                    &body,
                    properties,
                )
                .await
                .map_err(|e| Error::Protocol(format!("AMQP publish failed: {}", e)))?
                .await
                .map_err(|e| Error::Protocol(format!("AMQP publish confirm failed: {}", e)))?;

            connection
                .close(200, "publish complete")
                .await
                .map_err(|e| Error::Protocol(format!("AMQP close failed: {}", e)))?;

            Ok(())
        };

        tokio::time::timeout(Duration::from_secs(cfg.timeout_secs), publish)
            .await
            .map_err(|_| Error::Protocol(format!(
                "AMQP publish timed out after {}s",
                cfg.timeout_secs
            )))?
    }
}

impl Default for AmqpGateway {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run_consumer_listener(config: AmqpConsumeListenerConfig, shutdown: Arc<Notify>) {
    let connection = match Connection::connect(&config.broker_url, ConnectionProperties::default()).await {
        Ok(connection) => connection,
        Err(e) => {
            tracing::error!("AMQP consumer listener connect failed: {}", e);
            return;
        }
    };

    let channel = match connection.create_channel().await {
        Ok(channel) => channel,
        Err(e) => {
            tracing::error!("AMQP consumer listener channel failed: {}", e);
            let _ = connection.close(500, "channel failure").await;
            return;
        }
    };

    let consumer_tag = config
        .consumer_tag
        .clone()
        .unwrap_or_else(|| next_consumer_tag());

    let mut consumer = match channel
        .basic_consume(
            &config.queue,
            &consumer_tag,
            BasicConsumeOptions {
                no_ack: config.auto_ack,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
    {
        Ok(consumer) => consumer,
        Err(e) => {
            tracing::error!("AMQP consumer listener subscribe failed: {}", e);
            let _ = connection.close(500, "subscribe failure").await;
            return;
        }
    };

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    loop {
        let delivery = tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("AMQP consumer listener shutting down: {}", config.forward_to);
                let _ = channel.close(200, "shutdown").await;
                let _ = connection.close(200, "shutdown").await;
                break;
            }
            delivery = consumer.next() => delivery,
        };

        let Some(delivery) = delivery else {
            break;
        };

        match delivery {
            Ok(delivery) => {
                let response = http_client
                    .post(&config.forward_to)
                    .header("content-type", "application/octet-stream")
                    .header("x-amqp-source", "iron-babel-amqp-listener")
                    .header("x-amqp-exchange", delivery.exchange.as_str())
                    .header("x-amqp-routing-key", delivery.routing_key.as_str())
                    .header("x-amqp-delivery-tag", delivery.delivery_tag.to_string())
                    .body(delivery.data.clone())
                    .send()
                    .await;

                let ok = matches!(response, Ok(ref resp) if resp.status().is_success());

                if !config.auto_ack {
                    let ack_result = if ok {
                        delivery.ack(BasicAckOptions::default()).await
                    } else {
                        delivery
                            .nack(BasicNackOptions {
                                requeue: true,
                                ..Default::default()
                            })
                            .await
                    };

                    if let Err(e) = ack_result {
                        tracing::warn!("AMQP delivery ack/nack failed: {}", e);
                    }
                }

                match response {
                    Ok(resp) => tracing::debug!(
                        "AMQP→HTTP forwarded queue '{}' to {} → {}",
                        config.queue,
                        config.forward_to,
                        resp.status()
                    ),
                    Err(e) => tracing::warn!(
                        "AMQP→HTTP forward of queue '{}' to {} failed: {}",
                        config.queue,
                        config.forward_to,
                        e
                    ),
                }
            }
            Err(e) => {
                tracing::error!("AMQP consumer listener delivery error: {}", e);
                break;
            }
        }
    }
}

fn next_consumer_tag() -> String {
    let suffix = CONSUMER_TAG_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("iron-babel-amqp-consumer-{}", suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_tags_are_unique() {
        let a = next_consumer_tag();
        let b = next_consumer_tag();
        assert_ne!(a, b);
        assert!(a.starts_with("iron-babel-amqp-consumer-"));
    }
}
