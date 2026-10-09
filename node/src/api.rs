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
use mesh_core::FileManifest;

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

#[derive(Serialize)]
pub struct LifecycleResponse {
    pub state: NodeLifecycleState,
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
