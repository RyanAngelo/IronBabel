use async_trait::async_trait;
use crate::core::{Request, Response, MiddlewareConfig};
use crate::error::{Result, Error};

/// Compares two secrets without an early exit on the first differing byte.
///
/// A plain `==` returns as soon as bytes diverge, so the time taken reveals how
/// many leading bytes of a guess were correct — enough to recover a token one
/// byte at a time. Length is not hidden (and does not need to be).
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

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
                // `fold`, not `any`: short-circuiting on the first match would
                // make the number of comparisons depend on which key matched.
                let matched = keys.iter().fold(false, |acc, k| {
                    match k.as_str() {
                        Some(key) => acc | constant_time_eq(key, token),
                        None => acc,
                    }
                });

                if matched {
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

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_matches_identical_strings() {
        assert!(constant_time_eq("s3cret", "s3cret"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn constant_time_eq_rejects_differences() {
        assert!(!constant_time_eq("s3cret", "s3cres"));
        assert!(!constant_time_eq("s3cret", "S3cret"));
        assert!(!constant_time_eq("s3cret", "s3cret "));
        assert!(!constant_time_eq("s3cret", ""));
    }

    #[test]
    fn constant_time_eq_is_not_a_prefix_match() {
        assert!(!constant_time_eq("s3cret", "s3"));
        assert!(!constant_time_eq("s3", "s3cret"));
    }
}
