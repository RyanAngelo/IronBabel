use axum::{routing::get, Router};

use crate::admin::handlers::{
    admin_events, admin_health, admin_index, admin_metrics, admin_recent_requests, admin_routes,
};
use crate::core::gateway::AppState;

pub fn build_admin_router() -> Router<AppState> {
    Router::new()
        .route("/admin/", get(admin_index))
        .route("/admin/api/health", get(admin_health))
        .route("/admin/api/metrics", get(admin_metrics))
        .route("/admin/api/routes", get(admin_routes))
        .route("/admin/api/requests/recent", get(admin_recent_requests))
        .route("/admin/events", get(admin_events))
}
