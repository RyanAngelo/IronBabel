use serde::{Deserialize, Serialize};
use std::future::Future;

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

pub trait SchemaManager: Send + Sync {
    fn discover_schema(&self, endpoint: &str) -> impl Future<Output = Result<Schema>> + Send;
    fn generate_schema(&self, protocol: &str, content: &str) -> impl Future<Output = Result<Schema>> + Send;
}

pub trait SchemaGenerator {
    fn discover_schema(&self, endpoint: &str) -> impl Future<Output = Result<Schema>> + Send;
    fn generate_schema(&self, protocol: &str, content: &str) -> impl Future<Output = Result<Schema>> + Send;
} 