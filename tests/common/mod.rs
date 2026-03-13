use std::sync::Arc;
use tokio::sync::Mutex;

use iron_babel::config::RouteConfig;
use iron_babel::error::Result;
use iron_babel::protocols::Protocol;
use iron_babel::gateway::ProtocolGateway;

/// Build a `RouteConfig` with specific allowed methods (empty = allow all).
#[allow(dead_code)]
pub fn make_route_with_methods(path: &str, target: &str, methods: &[&str]) -> RouteConfig {
    RouteConfig {
        path: path.to_string(),
        target: target.to_string(),
        methods: methods.iter().map(|s| s.to_string()).collect(),
        timeout_secs: Some(5),
        strip_prefix: None,
    }
}

/// A mock protocol for testing
pub struct MockProtocol {
    name: String,
    encode_calls: Arc<Mutex<Vec<Vec<u8>>>>,
    decode_calls: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MockProtocol {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            encode_calls: Arc::new(Mutex::new(Vec::new())),
            decode_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn get_encode_calls(&self) -> Vec<Vec<u8>> {
        self.encode_calls.lock().await.clone()
    }

    pub async fn get_decode_calls(&self) -> Vec<Vec<u8>> {
        self.decode_calls.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl Protocol for MockProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        self.encode_calls.lock().await.push(data.clone());
        Ok(data)
    }

    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        self.decode_calls.lock().await.push(data.clone());
        Ok(data)
    }
}

/// A mock gateway for testing
pub struct MockGateway {
    protocol: Arc<MockProtocol>,
}

impl MockGateway {
    pub fn new(protocol: Arc<MockProtocol>) -> Self {
        Self { protocol }
    }
}

#[async_trait::async_trait]
impl ProtocolGateway for MockGateway {
    async fn handle_request(&self, request: Vec<u8>) -> Result<Vec<u8>> {
        Ok(request)
    }

    fn protocol(&self) -> Arc<dyn Protocol> {
        self.protocol.clone()
    }
}

/// Helper function to create test data
pub fn create_test_data(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 256) as u8).collect()
} 