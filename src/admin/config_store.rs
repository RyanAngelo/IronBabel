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
}
