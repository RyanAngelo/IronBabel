use async_trait::async_trait;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::Mutex;
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::Result;

pub struct RateLimitMiddleware {
    config: MiddlewareConfig,
    limits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimitMiddleware {
    pub fn new(config: MiddlewareConfig) -> Self {
        Self {
            config,
            limits: Mutex::new(HashMap::new()),
        }
    }

    fn is_rate_limited(&self, key: &str, limit: u32, window: Duration) -> bool {
        let mut limits = self.limits.lock().unwrap();
        let now = Instant::now();
        let timestamps = limits.entry(key.to_string()).or_insert_with(Vec::new);
        
        // Remove old timestamps
        timestamps.retain(|&t| now.duration_since(t) <= window);
        
        // Check if we're over the limit
        if timestamps.len() >= limit as usize {
            return true;
        }
        
        // Add new timestamp
        timestamps.push(now);
        false
    }
}

#[async_trait]
impl super::Middleware for RateLimitMiddleware {
    async fn handle_request(&self, request: Request) -> Result<Request> {
        // TODO: Implement rate limiting logic
        Ok(request)
    }

    async fn handle_response(&self, response: Response) -> Result<Response> {
        Ok(response)
    }

    fn config(&self) -> &MiddlewareConfig {
        &self.config
    }
} 