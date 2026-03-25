use axum::{routing::{get, post, put}, Router};

use crate::admin::handlers::{
    admin_config, admin_config_schema, admin_events, admin_health, admin_index, admin_metrics,
    admin_recent_requests, admin_routes, admin_save_draft_config, admin_validate_config,
};
use crate::core::gateway::AppState;

pub fn build_admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/", get(admin_index))
        .route("/admin/api/health", get(admin_health))
        .route("/admin/api/metrics", get(admin_metrics))
        .route("/admin/api/routes", get(admin_routes))
        .route("/admin/api/requests/recent", get(admin_recent_requests))
        .route("/admin/api/config", get(admin_config))
        .route("/admin/api/config/schema", get(admin_config_schema))
        .route("/admin/api/config/validate", post(admin_validate_config))
        .route("/admin/api/config/draft", put(admin_save_draft_config))
        .route("/admin/events", get(admin_events))
}
