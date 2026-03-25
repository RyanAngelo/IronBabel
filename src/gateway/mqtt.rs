use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Outgoing, Packet, QoS, Transport};
use tokio::sync::Notify;

use crate::config::{MqttSubListenerConfig, MqttTransportConfig};
use crate::error::{Error, Result};

static CLIENT_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrokerEndpoint {
    host: String,
    port: u16,
    tls: bool,
}

pub struct MqttGateway;

impl MqttGateway {
    pub fn new() -> Self {
        Self
    }

    pub async fn publish(&self, cfg: &MqttTransportConfig, body: Vec<u8>) -> Result<()> {
        let endpoint = parse_broker_url(&cfg.broker_url)?;
        let qos = qos_from_u8(cfg.qos)?;
        let client_id = cfg
            .client_id
            .clone()
            .unwrap_or_else(|| next_client_id("iron-babel-pub"));

        let mut options = mqtt_options(&endpoint, &client_id);
        options.set_keep_alive(Duration::from_secs(5));

        let publish = async move {
            let (client, mut eventloop) = AsyncClient::new(options, 10);

            client
                .publish(cfg.topic.clone(), qos, cfg.retain, body)
                .await
                .map_err(|e| Error::Protocol(format!("MQTT publish failed: {}", e)))?;
            client
                .disconnect()
                .await
                .map_err(|e| Error::Protocol(format!("MQTT disconnect failed: {}", e)))?;

            loop {
                match eventloop.poll().await {
                    Ok(Event::Outgoing(Outgoing::Disconnect)) => return Ok(()),
                    Ok(_) => {}
                    Err(e) => {
                        return Err(Error::Protocol(format!(
                            "MQTT event loop failed during publish: {}",
                            e
                        )));
                    }
                }
            }
        };

        tokio::time::timeout(Duration::from_secs(cfg.timeout_secs), publish)
            .await
            .map_err(|_| {
                Error::Protocol(format!(
                    "MQTT publish timed out after {}s",
                    cfg.timeout_secs
                ))
            })?
    }
}

impl Default for MqttGateway {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn run_sub_listener(config: MqttSubListenerConfig, shutdown: Arc<Notify>) {
    let endpoint = match parse_broker_url(&config.broker_url) {
        Ok(endpoint) => endpoint,
        Err(e) => {
            tracing::error!("MQTT subscriber listener config error: {}", e);
            return;
        }
    };

    let qos = match qos_from_u8(config.qos) {
        Ok(qos) => qos,
        Err(e) => {
            tracing::error!("MQTT subscriber listener config error: {}", e);
            return;
        }
    };

    let client_id = config
        .client_id
        .clone()
        .unwrap_or_else(|| next_client_id("iron-babel-sub"));
    let mut options = mqtt_options(&endpoint, &client_id);
    options.set_keep_alive(Duration::from_secs(5));

    let (client, mut eventloop) = AsyncClient::new(options, 100);
    for topic in &config.topics {
        if let Err(e) = client.subscribe(topic.clone(), qos).await {
            tracing::error!("MQTT subscribe to '{}' failed: {}", topic, e);
            let _ = client.disconnect().await;
            return;
        }
    }

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    loop {
        let event = tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("MQTT subscriber listener shutting down: {}", config.forward_to);
                let _ = client.disconnect().await;
                break;
            }
            event = eventloop.poll() => event,
        };

        match event {
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                match http_client
                    .post(&config.forward_to)
                    .header("content-type", "application/octet-stream")
                    .header("x-mqtt-source", "iron-babel-mqtt-listener")
                    .header("x-mqtt-topic", publish.topic.clone())
                    .header("x-mqtt-qos", qos_to_u8(publish.qos).to_string())
                    .header("x-mqtt-retain", if publish.retain { "true" } else { "false" })
                    .body(publish.payload.to_vec())
                    .send()
                    .await
                {
                    Ok(resp) => tracing::debug!(
                        "MQTT→HTTP forwarded topic '{}' to {} → {}",
                        publish.topic,
                        config.forward_to,
                        resp.status()
                    ),
                    Err(e) => tracing::warn!(
                        "MQTT→HTTP forward of topic '{}' to {} failed: {}",
                        publish.topic,
                        config.forward_to,
                        e
                    ),
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("MQTT subscriber listener error: {}", e);
                break;
            }
        }
    }
}

fn mqtt_options(endpoint: &BrokerEndpoint, client_id: &str) -> MqttOptions {
    let mut options = MqttOptions::new(client_id, endpoint.host.clone(), endpoint.port);
    if endpoint.tls {
        options.set_transport(Transport::tls_with_default_config());
    }
    options
}

fn next_client_id(prefix: &str) -> String {
    let suffix = CLIENT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", prefix, suffix)
}

fn qos_from_u8(qos: u8) -> Result<QoS> {
    match qos {
        0 => Ok(QoS::AtMostOnce),
        1 => Ok(QoS::AtLeastOnce),
        2 => Ok(QoS::ExactlyOnce),
        _ => Err(Error::Config("MQTT QoS must be 0, 1, or 2".to_string())),
    }
}

fn qos_to_u8(qos: QoS) -> u8 {
    match qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 1,
        QoS::ExactlyOnce => 2,
    }
}

fn parse_broker_url(url: &str) -> Result<BrokerEndpoint> {
    let (tls, rest) = if let Some(rest) = url.strip_prefix("mqtt://") {
        (false, rest)
    } else if let Some(rest) = url.strip_prefix("tcp://") {
        (false, rest)
    } else if let Some(rest) = url.strip_prefix("mqtts://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("ssl://") {
        (true, rest)
    } else {
        return Err(Error::Config(format!(
            "MQTT broker URL '{}' must start with mqtt://, mqtts://, tcp://, or ssl://",
            url
        )));
    };

    if rest.is_empty() {
        return Err(Error::Config("MQTT broker URL must include a host".to_string()));
    }
    if rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return Err(Error::Config(format!(
            "MQTT broker URL '{}' must be in scheme://host:port form",
            url
        )));
    }

    let (host, port) = split_host_port(rest).ok_or_else(|| {
        Error::Config(format!(
            "MQTT broker URL '{}' must include an explicit host and port",
            url
        ))
    })?;

    Ok(BrokerEndpoint { host, port, tls })
}

fn split_host_port(input: &str) -> Option<(String, u16)> {
    if let Some(rest) = input.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = rest[..end].to_string();
        let port_str = rest[end + 1..].strip_prefix(':')?;
        let port = port_str.parse().ok()?;
        return Some((host, port));
    }

    let idx = input.rfind(':')?;
    let host = input[..idx].to_string();
    let port = input[idx + 1..].parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_broker_url() {
        assert_eq!(
            parse_broker_url("mqtt://broker.local:1883").unwrap(),
            BrokerEndpoint {
                host: "broker.local".to_string(),
                port: 1883,
                tls: false,
            }
        );
    }

    #[test]
    fn parses_tls_broker_url() {
        assert_eq!(
            parse_broker_url("mqtts://broker.local:8883").unwrap(),
            BrokerEndpoint {
                host: "broker.local".to_string(),
                port: 8883,
                tls: true,
            }
        );
    }

    #[test]
    fn rejects_broker_url_without_port() {
        assert!(parse_broker_url("mqtt://broker.local").is_err());
    }

    #[test]
    fn converts_qos_levels() {
        assert_eq!(qos_from_u8(0).unwrap(), QoS::AtMostOnce);
        assert_eq!(qos_from_u8(1).unwrap(), QoS::AtLeastOnce);
        assert_eq!(qos_from_u8(2).unwrap(), QoS::ExactlyOnce);
        assert!(qos_from_u8(3).is_err());
    }
}
