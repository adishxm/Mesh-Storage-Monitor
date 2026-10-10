use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tower_http::cors::CorsLayer;

use crate::network::Command;
use crate::state::{NodeLifecycleState, NodeState, NodeStatus};
use mesh_core::{FileManifest, Invitation};

#[derive(Clone)]
pub struct AppState {
    pub node_state: Arc<RwLock<NodeState>>,
    pub network_tx: mpsc::Sender<Command>,
}

#[derive(Deserialize, Default)]
pub struct DownloadPayload {
    pub passphrase: Option<String>,
    pub salt: Option<String>,
    pub k: Option<usize>,
    pub m: Option<usize>,
}

pub fn check_api_auth_with_config(
    headers: &axum::http::HeaderMap,
    expected_key: &str,
    require_auth: bool,
    env_mode: &str,
) -> Result<(), (StatusCode, String)> {
    let auth_mandatory = require_auth
        || env_mode.eq_ignore_ascii_case("production")
        || env_mode.eq_ignore_ascii_case("lan")
        || !expected_key.is_empty();

    if auth_mandatory {
        if expected_key.is_empty() {
            return Err((
                StatusCode::UNAUTHORIZED,
                "Unauthorized: MESH_API_KEY must be configured in non-development modes"
                    .to_string(),
            ));
        }

        let provided = headers
            .get("x-mesh-api-key")
            .and_then(|v| v.to_str().ok())
            .or_else(|| {
                headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
            });

        if provided != Some(expected_key) {
            return Err((
                StatusCode::UNAUTHORIZED,
                "Unauthorized: Invalid or missing X-Mesh-Api-Key".to_string(),
            ));
        }
    }
    Ok(())
}

fn check_api_auth(headers: &axum::http::HeaderMap) -> Result<(), (StatusCode, String)> {
    let expected_key = std::env::var("MESH_API_KEY").unwrap_or_default();
    let require_auth = std::env::var("MESH_REQUIRE_AUTH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let env_mode = std::env::var("MESH_ENV").unwrap_or_else(|_| "development".to_string());

    check_api_auth_with_config(headers, &expected_key, require_auth, &env_mode)
}

#[derive(Deserialize)]
pub struct PairRequest {
    pub multiaddr: String,
}

#[derive(Deserialize)]
pub struct QuotaUpdateRequest {
    pub quota_bytes: Option<u64>,
    pub quota_gb: Option<f64>,
}

#[derive(Serialize)]
pub struct QuotaResponse {
    pub storage_used: u64,
    pub storage_quota: u64,
    pub usage_ratio: f64,
    pub remaining_bytes: u64,
}

#[derive(Deserialize)]
pub struct BandwidthLimitRequest {
    pub limit_kbps: Option<u64>,
}

#[derive(Serialize)]
pub struct BandwidthLimitResponse {
    pub limit_kbps: Option<u64>,
    pub success: bool,
}

#[derive(Serialize)]
pub struct LifecycleResponse {
    pub state: NodeLifecycleState,
    pub message: String,
}

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub network_id: Option<String>,
    pub organization_id: Option<String>,
    pub duration_secs: Option<u64>,
}

#[derive(Serialize)]
pub struct CreateInviteResponse {
    pub invitation: Invitation,
    pub qr_payload: String,
}

#[derive(Deserialize)]
pub struct JoinInviteRequest {
    pub invitation: Option<Invitation>,
    pub qr_payload: Option<String>,
}

#[derive(Serialize)]
pub struct JoinInviteResponse {
    pub success: bool,
    pub issuer_peer_id: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct ReliabilityResponse {
    pub peer_id: String,
    pub score: f64,
    pub is_healthy: bool,
}

#[derive(Serialize)]
pub struct RepairCheckResponse {
    pub file_id: String,
    pub degraded_chunks: Vec<mesh_core::DegradedChunk>,
    pub can_repair_all: bool,
}

#[derive(Serialize)]
pub struct MyCreditsResponse {
    pub peer_id: String,
    pub tier: mesh_core::ReciprocityTier,
    pub bytes_contributed: u64,
    pub bytes_consumed: u64,
    pub earned_allowance_bytes: u64,
    pub credit_balance: i64,
    pub fair_share_ratio: f64,
    pub audits_passed: u64,
    pub audits_failed: u64,
    pub uptime_seconds: u64,
}

#[derive(Serialize)]
pub struct PeerCreditsResponse {
    pub peer_id: String,
    pub tier: mesh_core::ReciprocityTier,
    pub bytes_contributed: u64,
    pub bytes_consumed: u64,
    pub credit_balance: i64,
    pub fair_share_ratio: f64,
    pub is_throttled: bool,
}

#[derive(Deserialize)]
pub struct BackupExportRequest {
    pub passphrase: String,
}

#[derive(Serialize)]
pub struct BackupExportResponse {
    pub archive_hex: String,
    pub bytes: usize,
    pub created_at_secs: u64,
}

#[derive(Deserialize)]
pub struct BackupRestoreRequest {
    pub passphrase: String,
    pub archive_hex: String,
}

#[derive(Serialize)]
pub struct BackupRestoreResponse {
    pub success: bool,
    pub restored_manifests: usize,
    pub restored_peers: usize,
    pub storage_quota: u64,
}

pub fn make_router(state: AppState) -> Router {
    let allowed_origins = [
        "http://localhost:3000"
            .parse::<axum::http::HeaderValue>()
            .unwrap(),
        "http://127.0.0.1:3000"
            .parse::<axum::http::HeaderValue>()
            .unwrap(),
        "http://[::1]:3000"
            .parse::<axum::http::HeaderValue>()
            .unwrap(),
    ];
    let mut cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-mesh-api-key"),
            axum::http::HeaderName::from_static("x-mesh-passphrase"),
            axum::http::HeaderName::from_static("x-mesh-salt"),
        ]);

    if let Some(val) = std::env::var("MESH_ALLOWED_ORIGIN")
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
                axum::http::HeaderName::from_static("x-mesh-api-key"),
                axum::http::HeaderName::from_static("x-mesh-passphrase"),
                axum::http::HeaderName::from_static("x-mesh-salt"),
            ]);
    }

    Router::new()
        // API v1 Canonical Endpoints
        .route("/api/v1/status", get(get_status))
        .route("/api/v1/peers", get(get_peers))
        .route("/api/v1/shards", get(get_shards))
        .route("/api/v1/manifests", get(get_manifests))
        .route("/api/v1/quota", get(get_quota).post(set_quota))
        .route("/api/v1/bandwidth", get(get_bandwidth).post(set_bandwidth))
        .route("/api/v1/invite/create", post(create_invite))
        .route("/api/v1/invite/join", post(join_invite))
        .route("/api/v1/pause", post(pause_node))
        .route("/api/v1/resume", post(resume_node))
        .route("/api/v1/leave", post(leave_node))
        .route("/api/v1/pair", post(pair_peer))
        .route("/api/v1/upload", post(upload_file))
        .route(
            "/api/v1/download/:file_id",
            post(download_file_post).get(download_file_get),
        )
        .route("/api/v1/reliability/:peer_id", get(get_peer_reliability))
        .route("/api/v1/repair/check/:file_id", get(check_file_repair))
        .route("/api/v1/credits/me", get(get_my_credits))
        .route("/api/v1/credits/peers/:peer_id", get(get_peer_credits))
        // Observability and Disaster Recovery Endpoints
        .route("/api/v1/metrics", get(get_metrics))
        .route("/metrics", get(get_metrics))
        .route("/api/v1/backup/export", post(export_backup))
        .route("/api/v1/backup/restore", post(restore_backup))
        // Backward-compatibility aliases for local prototypes & dashboards
        .route("/status", get(get_status))
        .route("/peers", get(get_peers))
        .route("/shards", get(get_shards))
        .route("/pair", post(pair_peer))
        .route("/upload", post(upload_file))
        .route(
            "/download/:file_id",
            post(download_file_post).get(download_file_get),
        )
        // Web Dashboard UI
        .route("/", get(serve_dashboard))
        .route("/dashboard", get(serve_dashboard))
        .with_state(state)
        .layer(cors)
}

async fn serve_dashboard() -> impl IntoResponse {
    Html(include_str!("../../dashboard.html"))
}

async fn get_status(State(state): State<AppState>) -> Json<NodeStatus> {
    let node_state = state.node_state.read().await;
    Json(node_state.get_status())
}

async fn get_peers(State(state): State<AppState>) -> Json<Vec<String>> {
    let node_state = state.node_state.read().await;
    Json(node_state.get_status().peers)
}

async fn get_shards(State(state): State<AppState>) -> Json<Vec<String>> {
    let node_state = state.node_state.read().await;
    Json(node_state.get_status().shards)
}

async fn get_manifests(State(state): State<AppState>) -> Json<Vec<FileManifest>> {
    let node_state = state.node_state.read().await;
    Json(node_state.list_manifests())
}

async fn get_quota(State(state): State<AppState>) -> Json<QuotaResponse> {
    let node_state = state.node_state.read().await;
    Json(QuotaResponse {
        storage_used: node_state.storage_used,
        storage_quota: node_state.storage_quota,
        usage_ratio: node_state.quota_tracker.usage_ratio(),
        remaining_bytes: node_state.quota_tracker.remaining_bytes(),
    })
}

async fn set_quota(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<QuotaUpdateRequest>,
) -> Result<Json<QuotaResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let mut node_state = state.node_state.write().await;
    let new_quota = if let Some(bytes) = payload.quota_bytes {
        bytes
    } else if let Some(gb) = payload.quota_gb {
        (gb * 1024.0 * 1024.0 * 1024.0).round() as u64
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "quota_bytes or quota_gb must be specified".to_string(),
        ));
    };

    node_state.set_quota(new_quota);
    Ok(Json(QuotaResponse {
        storage_used: node_state.storage_used,
        storage_quota: node_state.storage_quota,
        usage_ratio: node_state.quota_tracker.usage_ratio(),
        remaining_bytes: node_state.quota_tracker.remaining_bytes(),
    }))
}

async fn get_bandwidth(State(state): State<AppState>) -> Json<BandwidthLimitResponse> {
    let node_state = state.node_state.read().await;
    let limit_kbps = node_state
        .bandwidth_limiter
        .as_ref()
        .map(|l| l.max_rate_bytes_per_sec / 1024);
    Json(BandwidthLimitResponse {
        limit_kbps,
        success: true,
    })
}

async fn set_bandwidth(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<BandwidthLimitRequest>,
) -> Result<(StatusCode, Json<BandwidthLimitResponse>), (StatusCode, String)> {
    check_api_auth(&headers)?;
    let mut node_state = state.node_state.write().await;
    node_state.set_bandwidth_limit(payload.limit_kbps);
    Ok((
        StatusCode::OK,
        Json(BandwidthLimitResponse {
            limit_kbps: payload.limit_kbps,
            success: true,
        }),
    ))
}

async fn pause_node(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<LifecycleResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let mut node_state = state.node_state.write().await;
    node_state
        .pause()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(LifecycleResponse {
        state: node_state.state,
        message: "Node paused successfully".to_string(),
    }))
}

async fn resume_node(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<LifecycleResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let mut node_state = state.node_state.write().await;
    node_state
        .resume()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(LifecycleResponse {
        state: node_state.state,
        message: "Node resumed successfully".to_string(),
    }))
}

async fn leave_node(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<LifecycleResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let mut node_state = state.node_state.write().await;
    node_state
        .leave()
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(LifecycleResponse {
        state: node_state.state,
        message: "Node departure initiated. Shard repair handoff started.".to_string(),
    }))
}

async fn create_invite(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<CreateInviteRequest>,
) -> Result<Json<CreateInviteResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let node_state = state.node_state.read().await;
    let network_id = payload
        .network_id
        .unwrap_or_else(|| "mesh-alpha".to_string());
    let organization_id = payload
        .organization_id
        .unwrap_or_else(|| "org_default".to_string());
    let duration_secs = payload.duration_secs.unwrap_or(86400);

    let bootstrap_addrs = node_state
        .listen_addresses
        .iter()
        .map(|addr| format!("{}/p2p/{}", addr, node_state.peer_id))
        .collect();

    let invitation = crate::identity::create_signed_invitation(
        &node_state.data_dir,
        network_id,
        organization_id,
        node_state.peer_id.to_string(),
        bootstrap_addrs,
        duration_secs,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let qr_payload = invitation
        .to_qr_string()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(CreateInviteResponse {
        invitation,
        qr_payload,
    }))
}

async fn join_invite(
    State(state): State<AppState>,
    Json(payload): Json<JoinInviteRequest>,
) -> Result<Json<JoinInviteResponse>, (StatusCode, String)> {
    let invite = if let Some(inv) = payload.invitation {
        inv
    } else if let Some(qr) = payload.qr_payload {
        Invitation::from_qr_string(&qr).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid QR payload: {}", e),
            )
        })?
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Either invitation or qr_payload must be provided".to_string(),
        ));
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    invite.verify(now).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invitation verification failed: {}", e),
        )
    })?;

    {
        let mut node_state = state.node_state.write().await;
        node_state
            .consume_nonce(&invite.single_use_nonce)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

        if let Ok(issuer_peer_id) = invite.issuer_peer_id.parse::<libp2p::PeerId>() {
            node_state.add_trusted_peer(issuer_peer_id);
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid issuer_peer_id in invitation".to_string(),
            ));
        }
    }

    for addr in &invite.bootstrap_addrs {
        let (tx, _rx) = oneshot::channel();
        let _ = state
            .network_tx
            .send(Command::Pair {
                multiaddr: addr.clone(),
                response: tx,
            })
            .await;
    }

    Ok(Json(JoinInviteResponse {
        success: true,
        issuer_peer_id: invite.issuer_peer_id,
        message: "Invitation accepted. Issuer trusted and bootstrap dialed.".to_string(),
    }))
}

async fn pair_peer(
    State(state): State<AppState>,
    Json(payload): Json<PairRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let (tx, rx) = oneshot::channel();
    state
        .network_tx
        .send(Command::Pair {
            multiaddr: payload.multiaddr,
            response: tx,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    rx.await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn upload_file(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut file_id = None;
    let mut passphrase = None;
    let mut salt = None;
    let mut k = None;
    let mut m = None;
    let mut temp_path_opt: Option<std::path::PathBuf> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file_id" => file_id = Some(field.text().await.unwrap_or_default()),
            "passphrase" => passphrase = Some(field.text().await.unwrap_or_default()),
            "salt" => salt = Some(field.text().await.unwrap_or_default()),
            "k" => {
                k = Some(
                    field
                        .text()
                        .await
                        .unwrap_or_default()
                        .parse::<usize>()
                        .unwrap_or(2),
                )
            }
            "m" => {
                m = Some(
                    field
                        .text()
                        .await
                        .unwrap_or_default()
                        .parse::<usize>()
                        .unwrap_or(1),
                )
            }
            "file" => {
                use std::io::Write;
                let mut temp_file = tempfile::NamedTempFile::new().map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to create temporary upload file: {}", e),
                    )
                })?;
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
                {
                    temp_file
                        .write_all(&chunk)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                }
                temp_file
                    .flush()
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                let (_file, path) = temp_file.keep().map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to persist temp upload file: {}", e),
                    )
                })?;
                temp_path_opt = Some(path);
            }
            _ => {}
        }
    }

    let file_id = file_id.ok_or((StatusCode::BAD_REQUEST, "file_id required".to_string()))?;
    let passphrase =
        passphrase.ok_or((StatusCode::BAD_REQUEST, "passphrase required".to_string()))?;
    let salt = salt.ok_or((StatusCode::BAD_REQUEST, "salt required".to_string()))?;
    let k = k.ok_or((StatusCode::BAD_REQUEST, "k required".to_string()))?;
    let m = m.ok_or((StatusCode::BAD_REQUEST, "m required".to_string()))?;
    let temp_path =
        temp_path_opt.ok_or((StatusCode::BAD_REQUEST, "file data required".to_string()))?;

    let (tx, rx) = oneshot::channel();
    state
        .network_tx
        .send(Command::Upload {
            file_id,
            temp_file_path: Some(temp_path),
            data: None,
            passphrase: passphrase.into_bytes(),
            salt: salt.into_bytes(),
            k,
            m,
            response: tx,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let manifest = rx
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(manifest))
}

async fn download_file_post(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
    headers: axum::http::HeaderMap,
    payload: Option<Json<DownloadPayload>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let payload = payload.map(|Json(p)| p).unwrap_or_default();
    let passphrase = payload
        .passphrase
        .or_else(|| {
            headers
                .get("x-mesh-passphrase")
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
        })
        .ok_or((
            StatusCode::BAD_REQUEST,
            "passphrase required in request body or X-Mesh-Passphrase header".to_string(),
        ))?;

    let salt = payload
        .salt
        .or_else(|| {
            headers
                .get("x-mesh-salt")
                .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
        })
        .ok_or((
            StatusCode::BAD_REQUEST,
            "salt required in request body or X-Mesh-Salt header".to_string(),
        ))?;

    let k = payload.k.unwrap_or(2);
    let m = payload.m.unwrap_or(1);

    let (tx, rx) = oneshot::channel();
    state
        .network_tx
        .send(Command::Download {
            file_id,
            passphrase: passphrase.into_bytes(),
            salt: salt.into_bytes(),
            k,
            m,
            response: tx,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let file_data = rx
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(file_data)
}

async fn download_file_get(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
    headers: axum::http::HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if query.contains_key("passphrase") {
        return Err((
            StatusCode::BAD_REQUEST,
            "Security violation: Passphrases in query strings are rejected to prevent leakage in logs/history. Use POST /api/v1/download/:file_id with request body or pass X-Mesh-Passphrase header.".to_string(),
        ));
    }
    download_file_post(State(state), Path(file_id), headers, None).await
}

async fn get_peer_reliability(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> Json<ReliabilityResponse> {
    let node_state = state.node_state.read().await;
    let score = node_state.get_peer_reliability(&peer_id);
    let is_healthy = node_state.is_peer_healthy(&peer_id);
    Json(ReliabilityResponse {
        peer_id,
        score,
        is_healthy,
    })
}

async fn check_file_repair(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> Result<Json<RepairCheckResponse>, (StatusCode, String)> {
    let node_state = state.node_state.read().await;
    let manifest = node_state.read_manifest(&file_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Manifest {} not found", file_id),
        )
    })?;

    let degraded = node_state.check_manifest_health(&manifest);
    let can_repair_all = degraded.iter().all(|c| c.can_repair());

    Ok(Json(RepairCheckResponse {
        file_id,
        degraded_chunks: degraded,
        can_repair_all,
    }))
}

async fn get_my_credits(State(state): State<AppState>) -> Json<MyCreditsResponse> {
    let s = state.node_state.read().await;
    let base_free = 1_073_741_824; // 1 GB free base
    let ratio = 1.0;
    let cap = s.storage_quota * 2;
    let ledger = &s.local_credits;
    Json(MyCreditsResponse {
        peer_id: s.peer_id.to_string(),
        tier: ledger.evaluate_tier(base_free, ratio, cap),
        bytes_contributed: ledger.bytes_contributed,
        bytes_consumed: ledger.bytes_consumed,
        earned_allowance_bytes: ledger.earned_allowance_bytes(base_free, ratio, cap),
        credit_balance: ledger.credit_balance(base_free, ratio, cap),
        fair_share_ratio: ledger.fair_share_ratio(),
        audits_passed: ledger.audits_passed,
        audits_failed: ledger.audits_failed,
        uptime_seconds: ledger.uptime_seconds,
    })
}

async fn get_peer_credits(
    State(state): State<AppState>,
    Path(peer_id): Path<String>,
) -> Json<PeerCreditsResponse> {
    let s = state.node_state.read().await;
    let base_free = 1_073_741_824;
    let ratio = 1.0;
    let cap = 50_000_000_000;
    let ledger = s
        .peer_credits
        .get(&peer_id)
        .cloned()
        .unwrap_or_else(|| mesh_core::CreditLedger::new(peer_id.clone(), 0));
    let tier = ledger.evaluate_tier(base_free, ratio, cap);
    let is_throttled = tier == mesh_core::ReciprocityTier::Throttled
        || tier == mesh_core::ReciprocityTier::Suspended;
    Json(PeerCreditsResponse {
        peer_id,
        tier,
        bytes_contributed: ledger.bytes_contributed,
        bytes_consumed: ledger.bytes_consumed,
        credit_balance: ledger.credit_balance(base_free, ratio, cap),
        fair_share_ratio: ledger.fair_share_ratio(),
        is_throttled,
    })
}

async fn get_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let s = state.node_state.read().await;
    let base_free = 1_073_741_824;
    let ratio = 1.0;
    let cap = 50_000_000_000;
    let allowance = s
        .local_credits
        .earned_allowance_bytes(base_free, ratio, cap);
    let credit_bal = s.local_credits.credit_balance(base_free, ratio, cap);
    let limit_kbps = s
        .bandwidth_limiter
        .as_ref()
        .map(|l| l.max_rate_bytes_per_sec / 1024)
        .unwrap_or(0);

    let body = format!(
        r#"# HELP mesh_storage_used_bytes Locally used storage bytes
# TYPE mesh_storage_used_bytes gauge
mesh_storage_used_bytes {}

# HELP mesh_storage_quota_bytes Locally configured storage quota bytes
# TYPE mesh_storage_quota_bytes gauge
mesh_storage_quota_bytes {}

# HELP mesh_peers_connected Number of actively connected libp2p peers
# TYPE mesh_peers_connected gauge
mesh_peers_connected {}

# HELP mesh_shards_stored_total Total count of locally stored shards
# TYPE mesh_shards_stored_total gauge
mesh_shards_stored_total {}

# HELP mesh_reciprocity_contributed_bytes Total storage bytes contributed by this node
# TYPE mesh_reciprocity_contributed_bytes counter
mesh_reciprocity_contributed_bytes {}

# HELP mesh_reciprocity_consumed_bytes Total storage bytes consumed by this node
# TYPE mesh_reciprocity_consumed_bytes counter
mesh_reciprocity_consumed_bytes {}

# HELP mesh_reciprocity_allowance_bytes Earned reciprocal storage allowance
# TYPE mesh_reciprocity_allowance_bytes gauge
mesh_reciprocity_allowance_bytes {}

# HELP mesh_reciprocity_credit_balance Net credit balance in bytes
# TYPE mesh_reciprocity_credit_balance gauge
mesh_reciprocity_credit_balance {}

# HELP mesh_audit_challenges_passed_total Total count of passed proof-of-storage challenges
# TYPE mesh_audit_challenges_passed_total counter
mesh_audit_challenges_passed_total {}

# HELP mesh_audit_challenges_failed_total Total count of failed proof-of-storage challenges
# TYPE mesh_audit_challenges_failed_total counter
mesh_audit_challenges_failed_total {}

# HELP mesh_bandwidth_limit_kbps Bandwidth rate limit in KB/s (0 if unmetered)
# TYPE mesh_bandwidth_limit_kbps gauge
mesh_bandwidth_limit_kbps {}
"#,
        s.storage_used,
        s.storage_quota,
        s.connected_peers.len(),
        s.get_status().shards.len(),
        s.local_credits.bytes_contributed,
        s.local_credits.bytes_consumed,
        allowance,
        credit_bal,
        s.local_credits.audits_passed,
        s.local_credits.audits_failed,
        limit_kbps,
    );

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

async fn export_backup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<BackupExportRequest>,
) -> Result<Json<BackupExportResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let s = state.node_state.read().await;
    let snapshot = s.create_backup_snapshot();
    let archive = mesh_core::create_encrypted_backup(&snapshot, &payload.passphrase)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let bytes = archive.len();
    Ok(Json(BackupExportResponse {
        archive_hex: hex::encode(archive),
        bytes,
        created_at_secs: snapshot.created_at_secs,
    }))
}

async fn restore_backup(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<BackupRestoreRequest>,
) -> Result<Json<BackupRestoreResponse>, (StatusCode, String)> {
    check_api_auth(&headers)?;
    let archive_bytes = hex::decode(&payload.archive_hex)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid hex: {}", e)))?;
    let snapshot = mesh_core::restore_encrypted_backup(&archive_bytes, &payload.passphrase)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    let mut s = state.node_state.write().await;
    let (restored_manifests, restored_peers) = s.restore_backup_snapshot(snapshot);
    Ok(Json(BackupRestoreResponse {
        success: true,
        restored_manifests,
        restored_peers,
        storage_quota: s.storage_quota,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serve_dashboard_contains_title() {
        let resp = serve_dashboard().await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let html = String::from_utf8(body.to_vec()).expect("utf8 string");
        assert!(html.contains("MESH STORAGE MONITOR"));
        assert!(html.contains("Reciprocity Tier"));
        assert!(html.contains("Self-Healing Redundancy Inspector"));
    }

    #[tokio::test]
    async fn test_prometheus_metrics_endpoint() {
        let temp_dir = tempfile::tempdir().unwrap();
        let key = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = libp2p::PeerId::from(key.public());
        let node_state = NodeState::with_data_dir(peer_id, temp_dir.path().to_path_buf(), 2.0);
        let (tx, _rx) = mpsc::channel(1);
        let app_state = AppState {
            node_state: Arc::new(RwLock::new(node_state)),
            network_tx: tx,
        };

        let resp = get_metrics(State(app_state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let metrics_text = String::from_utf8(body.to_vec()).expect("utf8 string");
        assert!(metrics_text.contains("mesh_storage_used_bytes"));
        assert!(metrics_text.contains("mesh_storage_quota_bytes"));
        assert!(metrics_text.contains("mesh_reciprocity_contributed_bytes"));
    }

    #[tokio::test]
    async fn test_api_backup_export_and_restore_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let key = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = libp2p::PeerId::from(key.public());
        let mut node_state = NodeState::with_data_dir(peer_id, temp_dir.path().to_path_buf(), 5.0);
        node_state.record_peer_storage("12D3KooWRemoteNode", 4096, 2048);

        let (tx, _rx) = mpsc::channel(1);
        let app_state = AppState {
            node_state: Arc::new(RwLock::new(node_state)),
            network_tx: tx,
        };

        // Export backup
        let export_req = BackupExportRequest {
            passphrase: "secure_dr_passphrase_123".to_string(),
        };
        let export_resp = export_backup(
            State(app_state.clone()),
            axum::http::HeaderMap::new(),
            Json(export_req),
        )
        .await
        .expect("export ok");
        assert!(export_resp.bytes > 0);
        assert!(!export_resp.archive_hex.is_empty());

        // Restore backup into new fresh node
        let temp_dir2 = tempfile::tempdir().unwrap();
        let key2 = libp2p::identity::Keypair::generate_ed25519();
        let peer_id2 = libp2p::PeerId::from(key2.public());
        let fresh_node = NodeState::with_data_dir(peer_id2, temp_dir2.path().to_path_buf(), 0.5);
        let (tx2, _rx2) = mpsc::channel(1);
        let app_state2 = AppState {
            node_state: Arc::new(RwLock::new(fresh_node)),
            network_tx: tx2,
        };

        let restore_req = BackupRestoreRequest {
            passphrase: "secure_dr_passphrase_123".to_string(),
            archive_hex: export_resp.archive_hex.clone(),
        };
        let restore_resp = restore_backup(
            State(app_state2.clone()),
            axum::http::HeaderMap::new(),
            Json(restore_req),
        )
        .await
        .expect("restore ok");
        assert!(restore_resp.success);
        assert_eq!(restore_resp.restored_peers, 1);

        // Verify state is restored
        let restored_state = app_state2.node_state.read().await;
        assert!(
            restored_state
                .peer_credits
                .contains_key("12D3KooWRemoteNode")
        );
    }

    #[tokio::test]
    async fn test_reject_passphrase_in_query_params() {
        let temp_dir = tempfile::tempdir().unwrap();
        let key = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = libp2p::PeerId::from(key.public());
        let node_state = NodeState::with_data_dir(peer_id, temp_dir.path().to_path_buf(), 1.0);
        let (tx, _rx) = mpsc::channel(1);
        let app_state = AppState {
            node_state: Arc::new(RwLock::new(node_state)),
            network_tx: tx,
        };

        let mut query = std::collections::HashMap::new();
        query.insert("passphrase".to_string(), "leaked_in_url".to_string());
        query.insert("salt".to_string(), "00112233".to_string());

        let res = download_file_get(
            State(app_state),
            Path("test_file_id".to_string()),
            axum::http::HeaderMap::new(),
            Query(query),
        )
        .await;

        let err = res.err().expect("should return error");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(
            err.1
                .contains("Security violation: Passphrases in query strings are rejected")
        );
    }

    #[tokio::test]
    async fn test_check_api_auth_enforcement() {
        let key = "super_secret_admin_token";
        let mut headers = axum::http::HeaderMap::new();

        // Missing header -> Unauthorized
        let res1 = check_api_auth_with_config(&headers, key, false, "development");
        assert_eq!(res1.err().unwrap().0, StatusCode::UNAUTHORIZED);

        // Wrong header -> Unauthorized
        headers.insert("x-mesh-api-key", "wrong_token".parse().unwrap());
        let res2 = check_api_auth_with_config(&headers, key, false, "development");
        assert_eq!(res2.err().unwrap().0, StatusCode::UNAUTHORIZED);

        // Correct header -> OK
        headers.insert(
            "x-mesh-api-key",
            "super_secret_admin_token".parse().unwrap(),
        );
        let res3 = check_api_auth_with_config(&headers, key, false, "development");
        assert!(res3.is_ok());

        // Bearer token -> OK
        let mut bearer_headers = axum::http::HeaderMap::new();
        bearer_headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer super_secret_admin_token".parse().unwrap(),
        );
        let res4 = check_api_auth_with_config(&bearer_headers, key, false, "development");
        assert!(res4.is_ok());
    }

    #[tokio::test]
    async fn test_check_api_auth_mandatory_outside_dev() {
        let headers = axum::http::HeaderMap::new();
        // Mandatory when require_auth is true
        let res = check_api_auth_with_config(&headers, "", true, "development");
        assert_eq!(res.err().unwrap().0, StatusCode::UNAUTHORIZED);

        // Mandatory in lan mode
        let res_lan = check_api_auth_with_config(&headers, "", false, "lan");
        assert_eq!(res_lan.err().unwrap().0, StatusCode::UNAUTHORIZED);

        // Mandatory in production mode
        let res_prod = check_api_auth_with_config(&headers, "", false, "production");
        assert_eq!(res_prod.err().unwrap().0, StatusCode::UNAUTHORIZED);

        // Unauthenticated access permitted only in development mode when key is unconfigured
        let res_dev = check_api_auth_with_config(&headers, "", false, "development");
        assert!(res_dev.is_ok());
    }
}
