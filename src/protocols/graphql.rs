use async_trait::async_trait;
use serde_json::Value;
use crate::error::{Error, Result};
use super::Protocol;

pub struct GraphQLProtocol {
    name: String,
}

impl GraphQLProtocol {
    pub fn new(_settings: Value) -> Result<Self> {
        Ok(Self {
            name: "graphql".to_string(),
        })
    }
}

#[async_trait]
impl Protocol for GraphQLProtocol {
    fn name(&self) -> &str {
        &self.name
    }

    /// Validate that the payload is a JSON object containing a `query` field.
    /// Mutations and subscriptions are also expressed as `query`-keyed objects
    /// per the GraphQL-over-HTTP spec.
    async fn encode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        let value: Value = serde_json::from_slice(&data)
            .map_err(|e| Error::GraphQL(format!("Invalid JSON: {}", e)))?;

        if !value.is_object() {
            return Err(Error::GraphQL(
                "GraphQL request must be a JSON object".to_string(),
            ));
        }

        if value.get("query").is_none() {
            return Err(Error::GraphQL(
                "GraphQL request must contain a 'query' field".to_string(),
            ));
        }

        Ok(data)
    }

    /// Validate that the response is valid JSON (GraphQL always returns JSON).
    async fn decode(&self, data: Vec<u8>) -> Result<Vec<u8>> {
        if !data.is_empty() {
            serde_json::from_slice::<Value>(&data)
                .map_err(|e| Error::GraphQL(format!("GraphQL response is not valid JSON: {}", e)))?;
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proto() -> GraphQLProtocol {
        GraphQLProtocol::new(serde_json::Value::Null).unwrap()
    }

    #[tokio::test]
    async fn encode_accepts_query() {
        let p = proto();
        let body = br#"{"query":"{ users { id } }"}"#.to_vec();
        assert!(p.encode(body).await.is_ok());
    }

    #[tokio::test]
    async fn encode_accepts_mutation() {
        let p = proto();
        let body = br#"{"query":"mutation { createUser(name:\"bob\") { id } }","variables":{}}"#.to_vec();
        assert!(p.encode(body).await.is_ok());
    }

    #[tokio::test]
    async fn encode_rejects_missing_query_field() {
        let p = proto();
        let body = br#"{"operation":"{ users { id } }"}"#.to_vec();
        let err = p.encode(body).await.unwrap_err();
        assert!(err.to_string().contains("'query'"));
    }

    #[tokio::test]
    async fn encode_rejects_invalid_json() {
        let p = proto();
        let body = b"not json".to_vec();
        assert!(p.encode(body).await.is_err());
    }

    #[tokio::test]
    async fn encode_rejects_json_array() {
        let p = proto();
        let body = br#"[{"query":"{ id }"}]"#.to_vec();
        let err = p.encode(body).await.unwrap_err();
        assert!(err.to_string().contains("JSON object"));
    }

    #[tokio::test]
    async fn decode_accepts_valid_json() {
        let p = proto();
        let body = br#"{"data":{"users":[]}}"#.to_vec();
        assert!(p.decode(body).await.is_ok());
    }

    #[tokio::test]
    async fn decode_accepts_empty_body() {
        let p = proto();
        assert!(p.decode(vec![]).await.is_ok());
    }

    #[tokio::test]
    async fn decode_rejects_non_json_response() {
        let p = proto();
        let body = b"<html>error</html>".to_vec();
        assert!(p.decode(body).await.is_err());
    }
}
