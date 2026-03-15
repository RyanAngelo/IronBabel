use crate::config::RouteConfig;

/// Routes incoming requests to the appropriate backend based on configured path-prefix rules.
pub struct Router {
    /// Routes sorted by path length descending so longest (most specific) prefix wins.
    routes: Vec<RouteConfig>,
}

impl Router {
    /// Creates a new `Router` from a list of `RouteConfig` entries.
    /// Routes are sorted by path length (descending) so that longer prefixes
    /// take priority over shorter ones.
    pub fn new(mut routes: Vec<RouteConfig>) -> Self {
        routes.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        Self { routes }
    }

    /// Finds the best matching route for the given HTTP method and request path.
    ///
    /// Matching rules:
    /// - The route's `path` must be a prefix of `path`.
    /// - If the route's `methods` list is non-empty, `method` must appear in it
    ///   (case-insensitive).
    /// - If `method` contains non-ASCII or control characters, `None` is returned
    ///   immediately (security guard against malformed input).
    /// - Among all matching routes the longest prefix wins (guaranteed by sort order).
    pub fn route(&self, method: &str, path: &str) -> Option<&RouteConfig> {
        // Security: reject methods containing non-ASCII or control characters.
        if !method.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }

        let method_upper = method.to_ascii_uppercase();

        self.routes.iter().find(|route| {
            // Path-prefix check
            let prefix_matches = path.starts_with(route.path.as_str())
                // Ensure we don't match "/apiv2" with prefix "/api"
                && (path.len() == route.path.len()
                    || path[route.path.len()..].starts_with('/')
                    || route.path.ends_with('/'));

            if !prefix_matches {
                return false;
            }

            // Method check: empty list means allow any method.
            if route.methods.is_empty() {
                return true;
            }

            route.methods.iter().any(|m| m.to_ascii_uppercase() == method_upper)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_route(path: &str, methods: &[&str]) -> RouteConfig {
        RouteConfig {
            path: path.to_string(),
            target: "http://127.0.0.1:9000".to_string(),
            methods: methods.iter().map(|s| s.to_string()).collect(),
            timeout_secs: None,
            strip_prefix: None,
            zmq_pattern: None,
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let router = Router::new(vec![
            make_route("/api", &["GET"]),
            make_route("/api/v1", &["GET"]),
        ]);
        let route = router.route("GET", "/api/v1/users").unwrap();
        assert_eq!(route.path, "/api/v1");
    }

    #[test]
    fn no_partial_segment_match() {
        let router = Router::new(vec![make_route("/api", &["GET"])]);
        assert!(router.route("GET", "/apiv2").is_none());
    }

    #[test]
    fn rejects_invalid_method() {
        let router = Router::new(vec![make_route("/", &[])]);
        assert!(router.route("GET\r\nX-Injected: bad", "/").is_none());
    }

    #[test]
    fn empty_methods_allows_any() {
        let router = Router::new(vec![make_route("/health", &[])]);
        assert!(router.route("DELETE", "/health").is_some());
    }
}
