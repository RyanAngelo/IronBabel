use async_trait::async_trait;
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::{Result, Error};

pub struct AuthMiddleware {
    config: MiddlewareConfig,
}

impl AuthMiddleware {
    pub fn new(config: MiddlewareConfig) -> Self {
        Self { config }
    }

    fn validate_token(&self, token: &str) -> Result<()> {
        // TODO: Implement token validation
        Ok(())
    }
}

#[async_trait]
impl super::Middleware for AuthMiddleware {
    async fn handle_request(&self, request: Request) -> Result<Request> {
        // Check for authentication header
        if let Some(auth_header) = request.headers().get("Authorization") {
            if let Ok(token) = auth_header.to_str() {
                if token.starts_with("Bearer ") {
                    let token = &token[7..];
                    self.validate_token(token)?;
                }
            }
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