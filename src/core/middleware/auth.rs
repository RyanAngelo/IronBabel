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

    /// Validates `token` against the `api_keys` list in settings.
    /// If no keys are configured (or the list is empty), all tokens are accepted.
    fn validate_token(&self, token: &str) -> Result<()> {
        let api_keys = self.config.settings
            .get("api_keys")
            .and_then(|v| v.as_array());

        match api_keys {
            Some(keys) if !keys.is_empty() => {
                if keys.iter().any(|k| k.as_str() == Some(token)) {
                    Ok(())
                } else {
                    Err(Error::Unauthorized("Invalid token".to_string()))
                }
            }
            _ => Ok(()), // no keys configured → auth not enforced
        }
    }
}

#[async_trait]
impl super::Middleware for AuthMiddleware {
    async fn handle_request(&self, request: Request) -> Result<Request> {
        if !self.config.enabled {
            return Ok(request);
        }

        let auth_header = request.metadata.headers.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.as_str());

        let keys_configured = self.config.settings
            .get("api_keys")
            .and_then(|v| v.as_array())
            .map(|k| !k.is_empty())
            .unwrap_or(false);

        match auth_header {
            Some(value) if value.starts_with("Bearer ") => {
                self.validate_token(&value[7..])?;
            }
            None => {
                if keys_configured {
                    return Err(Error::Unauthorized(
                        "Missing Authorization header".to_string(),
                    ));
                }
            }
            Some(_) => {
                // Authorization header present but not Bearer scheme.
                // Reject when API keys are configured — silently passing a
                // non-Bearer credential would allow scheme-switching as an
                // auth bypass.
                if keys_configured {
                    return Err(Error::Unauthorized(
                        "Invalid Authorization scheme: Bearer required".to_string(),
                    ));
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
