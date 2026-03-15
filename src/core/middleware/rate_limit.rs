use async_trait::async_trait;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::{Result, Error};

pub struct RateLimitMiddleware {
    config: MiddlewareConfig,
    /// Per-key sliding-window timestamp store.
    limits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimitMiddleware {
    pub fn new(config: MiddlewareConfig) -> Self {
        Self {
            config,
            limits: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if `key` has exceeded `limit` requests within `window`.
    /// Automatically removes timestamps that are older than the window and
    /// records the current request if it is allowed.
    async fn is_rate_limited(&self, key: &str, limit: u32, window: Duration) -> bool {
        let mut limits = self.limits.lock().await;
        let now = Instant::now();
        let timestamps = limits.entry(key.to_string()).or_default();

        // Evict expired timestamps.
        timestamps.retain(|&t| now.duration_since(t) <= window);

        if timestamps.len() >= limit as usize {
            return true;
        }

        timestamps.push(now);
        false
    }
}

#[async_trait]
impl super::Middleware for RateLimitMiddleware {
    async fn handle_request(&self, request: Request) -> Result<Request> {
        if !self.config.enabled {
            return Ok(request);
        }

        let limit = self.config.settings
            .get("requests_per_window")
            .and_then(|v| v.as_u64())
            .unwrap_or(100) as u32;

        let window_secs = self.config.settings
            .get("window_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        // Use X-Forwarded-For as the per-client key; fall back to "global".
        let key = request.metadata.headers.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("x-forwarded-for"))
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "global".to_string());

        if self.is_rate_limited(&key, limit, Duration::from_secs(window_secs)).await {
            return Err(Error::RateLimited);
        }

        Ok(request)
    }

    async fn handle_response(&self, response: Response) -> Result<Response> {
        Ok(response)
    }

    fn config(&self) -> &MiddlewareConfig {
        &self.config
    }
}
