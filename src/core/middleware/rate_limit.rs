use async_trait::async_trait;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::{Result, Error};

/// Maximum number of distinct client keys tracked simultaneously.
/// When this threshold is exceeded, all keys with no recent requests are
/// evicted to reclaim memory before inserting a new entry.
const MAX_TRACKED_CLIENTS: usize = 10_000;

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
    ///
    /// When the total number of tracked clients exceeds `MAX_TRACKED_CLIENTS`,
    /// expired entries are evicted from all buckets before inserting a new key.
    async fn is_rate_limited(&self, key: &str, limit: u32, window: Duration) -> bool {
        let mut limits = self.limits.lock().await;
        let now = Instant::now();

        // Evict expired entries from all buckets when the map is too large.
        // This prevents unbounded memory growth from high-cardinality client keys.
        if !limits.contains_key(key) && limits.len() >= MAX_TRACKED_CLIENTS {
            limits.retain(|_, timestamps| {
                timestamps.retain(|&t| now.duration_since(t) <= window);
                !timestamps.is_empty()
            });
        }

        let timestamps = limits.entry(key.to_string()).or_default();

        // Evict expired timestamps for this key.
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

        // Prefer the verified remote IP from the TCP socket. This cannot be
        // spoofed by the client. Fall back to a shared "global" bucket only when
        // the address is genuinely unavailable (e.g. in unit tests).
        let key = request.metadata.remote_addr
            .clone()
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
