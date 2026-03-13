use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

pub mod discovery;
pub mod generation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    pub version: String,
    pub content: String,
    pub protocol: String,
}

#[async_trait]
pub trait SchemaManager: Send + Sync {
    async fn discover_schema(&self, endpoint: &str) -> Result<Schema>;
    async fn generate_schema(&self, protocol: &str, content: &str) -> Result<Schema>;
} 