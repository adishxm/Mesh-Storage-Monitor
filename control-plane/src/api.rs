use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use tower_http::cors::CorsLayer;

use crate::models::{Device, DeviceStatus, DeviceType, Invite, OidcClaims, Tenant, TenantMetrics};
use crate::service::{ControlPlaneService, ServiceError};

#[derive(Clone)]
pub struct ApiState {
    pub service: ControlPlaneService,
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            ServiceError::TenantNotFound(m) => (StatusCode::NOT_FOUND, m),
            ServiceError::DeviceNotFound(m) => (StatusCode::NOT_FOUND, m),
            ServiceError::InviteNotFound(m) => (StatusCode::NOT_FOUND, m),
            ServiceError::InviteExpired => (StatusCode::BAD_REQUEST, "Invite has expired".into()),
            ServiceError::InviteAlreadyConsumed => {
                (StatusCode::BAD_REQUEST, "Invite already consumed".into())
            }
            ServiceError::TenantIsolationViolation { .. } => (
                StatusCode::FORBIDDEN,
                "Cross-tenant access forbidden".into(),
            ),
            ServiceError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            ServiceError::QuotaExceeded { .. } => (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()),
            ServiceError::DuplicateSlug(m) => (StatusCode::CONFLICT, m),
            ServiceError::DuplicateDevice(m) => (StatusCode::CONFLICT, m),
            ServiceError::TenantInactive(m) => (StatusCode::FORBIDDEN, m),
            ServiceError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, Json(serde_json::json!({ "error": msg }))).into_response()
    }
}

pub fn extract_claims(headers: &HeaderMap) -> Result<OidcClaims, ServiceError> {
    crate::auth::authenticate_claims(headers)
}

#[derive(Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub slug: String,
    pub max_quota_bytes: u64,
}

#[derive(Deserialize)]
pub struct RegisterDeviceRequest {
    pub peer_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub quota_bytes: u64,
}

#[derive(Deserialize)]
pub struct HeartbeatRequest {
    pub storage_used_bytes: u64,
}

#[derive(Deserialize)]
pub struct UpdateDeviceStatusRequest {
    pub status: DeviceStatus,
}

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub duration_secs: u64,
}

#[derive(Deserialize)]
pub struct ConsumeInviteRequest {
    pub invitation_token: String,
    pub peer_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub quota_bytes: u64,
}

pub fn make_router(service: ControlPlaneService) -> Router {
    let loopback_origins = [
        "http://localhost:3000".parse().unwrap(),
        "http://127.0.0.1:3000".parse().unwrap(),
        "http://[::1]:3000".parse().unwrap(),
        "http://localhost:8080".parse().unwrap(),
        "http://127.0.0.1:8080".parse().unwrap(),
    ];
    let mut cors = CorsLayer::new()
        .allow_origin(loopback_origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-oidc-sub"),
            axum::http::HeaderName::from_static("x-oidc-email"),
            axum::http::HeaderName::from_static("x-oidc-tenant"),
            axum::http::HeaderName::from_static("x-oidc-roles"),
        ]);

    if let Some(val) = std::env::var("CONTROL_PLANE_ALLOWED_ORIGIN")
        .ok()
        .and_then(|orig| orig.parse::<axum::http::HeaderValue>().ok())
    {
        cors = CorsLayer::new()
            .allow_origin([val])
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
                axum::http::HeaderName::from_static("x-oidc-sub"),
                axum::http::HeaderName::from_static("x-oidc-email"),
                axum::http::HeaderName::from_static("x-oidc-tenant"),
                axum::http::HeaderName::from_static("x-oidc-roles"),
            ]);
    }

    let state = ApiState { service };

    Router::new()
        .route("/api/v1/control/tenants", post(create_tenant))
        .route("/api/v1/control/tenants/:tenant_id", get(get_tenant))
        .route(
            "/api/v1/control/tenants/:tenant_id/devices",
            get(list_devices).post(register_device),
        )
        .route(
            "/api/v1/control/tenants/:tenant_id/devices/:device_id/heartbeat",
            post(device_heartbeat),
        )
        .route(
            "/api/v1/control/tenants/:tenant_id/devices/:device_id/status",
            post(update_device_status),
        )
        .route(
            "/api/v1/control/tenants/:tenant_id/invites",
            post(create_invite),
        )
        .route("/api/v1/control/invites/consume", post(consume_invite))
        .route(
            "/api/v1/control/tenants/:tenant_id/metrics",
            get(get_tenant_metrics),
        )
        .route("/api/v1/control/credits/:peer_id", get(get_peer_credits))
        // Observability Metrics & Health Endpoints
        .route("/api/v1/control/metrics", get(get_control_metrics))
        .route("/metrics", get(get_control_metrics))
        .route("/api/v1/control/health", get(get_control_health))
        .route("/health", get(get_control_health))
        .with_state(state)
        .layer(cors)
}

async fn create_tenant(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateTenantRequest>,
) -> Result<(StatusCode, Json<Tenant>), ServiceError> {
    let claims = extract_claims(&headers)?;
    let tenant = state
        .service
        .create_tenant(&claims, req.name, req.slug, req.max_quota_bytes)
        .await?;
    Ok((StatusCode::CREATED, Json(tenant)))
}

async fn get_tenant(
    State(state): State<ApiState>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Tenant>, ServiceError> {
    let claims = extract_claims(&headers)?;
    let tenant = state.service.get_tenant(&claims, &tenant_id).await?;
    Ok(Json(tenant))
}

async fn register_device(
    State(state): State<ApiState>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<RegisterDeviceRequest>,
) -> Result<(StatusCode, Json<Device>), ServiceError> {
    let claims = extract_claims(&headers)?;
    let device = state
        .service
        .register_device(
            &claims,
            &tenant_id,
            &req.peer_id,
            &req.device_name,
            req.device_type,
            req.quota_bytes,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(device)))
}

async fn list_devices(
    State(state): State<ApiState>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<Device>>, ServiceError> {
    let claims = extract_claims(&headers)?;
    let devices = state.service.list_devices(&claims, &tenant_id).await?;
    Ok(Json(devices))
}

async fn device_heartbeat(
    State(state): State<ApiState>,
    Path((tenant_id, device_id)): Path<(String, String)>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<Device>, ServiceError> {
    let device = state
        .service
        .update_device_heartbeat(&tenant_id, &device_id, req.storage_used_bytes)
        .await?;
    Ok(Json(device))
}

async fn update_device_status(
    State(state): State<ApiState>,
    Path((tenant_id, device_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<UpdateDeviceStatusRequest>,
) -> Result<Json<Device>, ServiceError> {
    let claims = extract_claims(&headers)?;
    let device = state
        .service
        .update_device_status(&claims, &tenant_id, &device_id, req.status)
        .await?;
    Ok(Json(device))
}

async fn create_invite(
    State(state): State<ApiState>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<Invite>), ServiceError> {
    let claims = extract_claims(&headers)?;
    let invite = state
        .service
        .create_invite(&claims, &tenant_id, req.duration_secs)
        .await?;
    Ok((StatusCode::CREATED, Json(invite)))
}

async fn consume_invite(
    State(state): State<ApiState>,
    Json(req): Json<ConsumeInviteRequest>,
) -> Result<(StatusCode, Json<Device>), ServiceError> {
    let device = state
        .service
        .consume_invite(
            &req.invitation_token,
            &req.peer_id,
            &req.device_name,
            req.device_type,
            req.quota_bytes,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(device)))
}

async fn get_tenant_metrics(
    State(state): State<ApiState>,
    Path(tenant_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<TenantMetrics>, ServiceError> {
    let claims = extract_claims(&headers)?;
    let metrics = state
        .service
        .get_tenant_metrics(&claims, &tenant_id)
        .await?;
    Ok(Json(metrics))
}

async fn get_peer_credits(
    State(state): State<ApiState>,
    Path(peer_id): Path<String>,
) -> impl IntoResponse {
    let report = state.service.get_peer_credit_report(&peer_id).await;
    (StatusCode::OK, Json(report))
}

async fn get_control_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "UP",
        "service": "mesh-control-plane",
        "version": "0.1.0"
    }))
}

async fn get_control_metrics(State(state): State<ApiState>) -> impl IntoResponse {
    let (tenants_cnt, devices_cnt, total_quota, credits_cnt) =
        state.service.get_system_metrics().await;

    let body = format!(
        r#"# HELP mesh_control_tenants_total Total active organizations and tenants
# TYPE mesh_control_tenants_total gauge
mesh_control_tenants_total {}

# HELP mesh_control_devices_total Total registered devices across all tenants
# TYPE mesh_control_devices_total gauge
mesh_control_devices_total {}

# HELP mesh_control_allocated_quota_bytes Total quota allocated across all tenants
# TYPE mesh_control_allocated_quota_bytes gauge
mesh_control_allocated_quota_bytes {}

# HELP mesh_control_audited_peers_total Total peers with active credit ledgers
# TYPE mesh_control_audited_peers_total gauge
mesh_control_audited_peers_total {}
"#,
        tenants_cnt, devices_cnt, total_quota, credits_cnt
    );

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_control_health_endpoint() {
        let health = get_control_health().await;
        assert_eq!(health["status"], "UP");
        assert_eq!(health["service"], "mesh-control-plane");
    }

    #[tokio::test]
    async fn test_control_metrics_endpoint() {
        let service = ControlPlaneService::new();
        let state = ApiState { service };
        let resp = get_control_metrics(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let text = String::from_utf8(body.to_vec()).expect("utf8 string");
        assert!(text.contains("mesh_control_tenants_total"));
        assert!(text.contains("mesh_control_devices_total"));
        assert!(text.contains("mesh_control_allocated_quota_bytes"));
    }
}
