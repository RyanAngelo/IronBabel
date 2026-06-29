use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::GatewayConfig;

pub struct AdminConfigStore {
    active: Arc<RwLock<GatewayConfig>>,
    draft: Arc<RwLock<GatewayConfig>>,
}

impl AdminConfigStore {
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            active: Arc::new(RwLock::new(config.clone())),
            draft: Arc::new(RwLock::new(config)),
        }
    }

    pub async fn active(&self) -> GatewayConfig {
        self.active.read().await.clone()
    }

    pub async fn draft(&self) -> GatewayConfig {
        self.draft.read().await.clone()
    }

    pub async fn snapshot(&self) -> (GatewayConfig, GatewayConfig) {
        let active = self.active.read().await.clone();
        let draft = self.draft.read().await.clone();
        (active, draft)
    }

    pub async fn save_draft(&self, config: GatewayConfig) {
        *self.draft.write().await = config;
    }

    /// Promotes the current draft to active after re-validating it. Returns the
    /// newly-active config, or a validation error if the draft is invalid.
    ///
    /// NOTE: this updates the in-memory active config surfaced by the admin API.
    /// Live request routing is established at startup, so applying routing
    /// changes to running traffic still requires a restart.
    pub async fn activate_draft(&self) -> crate::error::Result<GatewayConfig> {
        let draft = self.draft.read().await.clone();
        draft.validate()?;
        *self.active.write().await = draft.clone();
        Ok(draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HttpTransportConfig, RouteConfig, TransportConfig};

    fn config_with_route(path: &str) -> GatewayConfig {
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 8080,
            protocols: vec![],
            routes: vec![RouteConfig {
                path: path.to_string(),
                methods: vec![],
                transport: TransportConfig::Http(HttpTransportConfig {
                    url: "http://127.0.0.1:9000".to_string(),
                    timeout_secs: 30,
                    strip_prefix: false,
                }),
            }],
            listeners: vec![],
            middleware: Default::default(),
        }
    }

    #[tokio::test]
    async fn activate_draft_promotes_valid_draft() {
        let store = AdminConfigStore::new(config_with_route("/old"));
        store.save_draft(config_with_route("/new")).await;

        let active = store.activate_draft().await.unwrap();
        assert_eq!(active.routes[0].path, "/new");
        assert_eq!(store.active().await.routes[0].path, "/new");
    }

    #[tokio::test]
    async fn activate_draft_rejects_invalid_draft() {
        let store = AdminConfigStore::new(config_with_route("/old"));
        let mut bad = config_with_route("/new");
        bad.port = 0; // invalid
        store.save_draft(bad).await;

        assert!(store.activate_draft().await.is_err());
        // Active config is unchanged when activation fails.
        assert_eq!(store.active().await.routes[0].path, "/old");
    }
}
