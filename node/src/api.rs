use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tower_http::cors::{Any, CorsLayer};

use crate::network::Command;
use crate::state::{NodeLifecycleState, NodeState, NodeStatus};
use mesh_core::{FileManifest, Invitation};

#[derive(Clone)]
pub struct AppState {
    pub node_state: Arc<RwLock<NodeState>>,
    pub network_tx: mpsc::Sender<Command>,
}

#[derive(Deserialize)]
pub struct DownloadQuery {
    pub passphrase: String,
    pub salt: String,
    pub k: usize,
    pub m: usize,
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

pub fn make_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

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
        .route("/api/v1/download/:file_id", get(download_file))
        // Backward-compatibility aliases for local prototypes & dashboards
        .route("/status", get(get_status))
        .route("/peers", get(get_peers))
        .route("/shards", get(get_shards))
        .route("/pair", post(pair_peer))
        .route("/upload", post(upload_file))
        .route("/download/:file_id", get(download_file))
        .with_state(state)
        .layer(cors)
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
    Json(payload): Json<QuotaUpdateRequest>,
) -> Result<Json<QuotaResponse>, (StatusCode, String)> {
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
    Json(payload): Json<BandwidthLimitRequest>,
) -> (StatusCode, Json<BandwidthLimitResponse>) {
    let mut node_state = state.node_state.write().await;
    node_state.set_bandwidth_limit(payload.limit_kbps);
    (
        StatusCode::OK,
        Json(BandwidthLimitResponse {
            limit_kbps: payload.limit_kbps,
            success: true,
        }),
    )
}

async fn pause_node(
    State(state): State<AppState>,
) -> Result<Json<LifecycleResponse>, (StatusCode, String)> {
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
) -> Result<Json<LifecycleResponse>, (StatusCode, String)> {
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
) -> Result<Json<LifecycleResponse>, (StatusCode, String)> {
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
    Json(payload): Json<CreateInviteRequest>,
) -> Result<Json<CreateInviteResponse>, (StatusCode, String)> {
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
    let mut data = None;

    while let Some(field) = multipart
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
                data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
                );
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
    let data = data.ok_or((StatusCode::BAD_REQUEST, "file data required".to_string()))?;

    let (tx, rx) = oneshot::channel();
    state
        .network_tx
        .send(Command::Upload {
            file_id,
            data: data.to_vec(),
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

async fn download_file(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
    Query(query): Query<DownloadQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (tx, rx) = oneshot::channel();
    state
        .network_tx
        .send(Command::Download {
            file_id,
            passphrase: query.passphrase.into_bytes(),
            salt: query.salt.into_bytes(),
            k: query.k,
            m: query.m,
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
