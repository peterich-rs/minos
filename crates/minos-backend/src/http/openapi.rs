//! OpenAPI spec generation and serving.
//!
//! Uses `utoipa` to derive an OpenAPI 3.1 spec from handler annotations
//! and schema derives. The spec is served at `GET /openapi.json`.

use utoipa::OpenApi;

/// The top-level OpenAPI document for the Minos backend.
///
/// All paths and schemas are registered here. Individual handler modules
/// add `#[utoipa::path(...)]` annotations; this struct aggregates them.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Minos Backend API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Minos backend server: WebSocket hub, auth, and REST management API.",
        contact(name = "Minos Team"),
        license(name = "Proprietary"),
    ),
    paths(
        // Auth
        crate::http::v1::auth::post_register,
        crate::http::v1::auth::post_login,
        crate::http::v1::auth::post_refresh,
        crate::http::v1::auth::post_logout,
        crate::http::v1::auth::post_change_password,
        // Health
        crate::http::health::live,
        crate::http::health::ready,
        crate::http::health::info,
    ),
    components(schemas(
        // Auth schemas
        crate::http::v1::auth::RegisterReq,
        crate::http::v1::auth::LoginReq,
        crate::http::v1::auth::RefreshReq,
        crate::http::v1::auth::LogoutReq,
        crate::http::v1::auth::AuthResp,
        crate::http::v1::auth::RefreshResp,
        crate::http::v1::auth::AccountSummary,
        // Error schemas
        crate::http::error_response::ErrorEnvelope,
        crate::http::error_response::ErrorBody,
    )),
    tags(
        (name = "auth", description = "Account registration, login, token refresh, logout"),
        (name = "agent-sessions", description = "Agent session lifecycle management"),
        (name = "health", description = "Health and readiness probes"),
        (name = "host", description = "Host (Mac) bootstrap and pairing"),
        (name = "pairing", description = "Account-host pairing management"),
        (name = "projects", description = "Project CRUD and agent-session linking"),
        (name = "social", description = "Conversations, friends, and messaging"),
        (name = "threads", description = "Thread listing and reading (DEPRECATED)"),
    )
)]
pub struct ApiDoc;

/// Serve the OpenAPI spec as JSON at `/openapi.json`.
pub async fn serve_openapi_json() -> axum::Json<utoipa::openapi::OpenApi> {
    axum::Json(ApiDoc::openapi().clone())
}
