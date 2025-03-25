use async_trait::async_trait;
use crate::error::Result;
use super::Transformer;

pub struct JsonTransformer;

#[async_trait]
impl Transformer for JsonTransformer {
    async fn transform(&self, input: Vec<u8>, _from: &str, _to: &str) -> Result<Vec<u8>> {
        Ok(input)
    }
} 