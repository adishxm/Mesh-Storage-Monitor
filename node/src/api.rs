use axum::{
    extract::{Path, Query, State, Multipart},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use serde::Deserialize;
use tower_http::cors::{Any, CorsLayer};

use crate::state::{NodeState, NodeStatus};
use crate::network::Command;

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

pub fn make_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
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

async fn pair_peer(
    State(state): State<AppState>,
    Json(payload): Json<PairRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let (tx, rx) = oneshot::channel();
    state.network_tx.send(Command::Pair {
        multiaddr: payload.multiaddr,
        response: tx,
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    rx.await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
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

    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file_id" => file_id = Some(field.text().await.unwrap_or_default()),
            "passphrase" => passphrase = Some(field.text().await.unwrap_or_default()),
            "salt" => salt = Some(field.text().await.unwrap_or_default()),
            "k" => k = Some(field.text().await.unwrap_or_default().parse::<usize>().unwrap_or(2)),
            "m" => m = Some(field.text().await.unwrap_or_default().parse::<usize>().unwrap_or(1)),
            "file" => {
                data = Some(field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?);
            }
            _ => {}
        }
    }

    let file_id = file_id.ok_or((StatusCode::BAD_REQUEST, "file_id required".to_string()))?;
    let passphrase = passphrase.ok_or((StatusCode::BAD_REQUEST, "passphrase required".to_string()))?;
    let salt = salt.ok_or((StatusCode::BAD_REQUEST, "salt required".to_string()))?;
    let k = k.ok_or((StatusCode::BAD_REQUEST, "k required".to_string()))?;
    let m = m.ok_or((StatusCode::BAD_REQUEST, "m required".to_string()))?;
    let data = data.ok_or((StatusCode::BAD_REQUEST, "file data required".to_string()))?;

    let (tx, rx) = oneshot::channel();
    state.network_tx.send(Command::Upload {
        file_id,
        data: data.to_vec(),
        passphrase: passphrase.into_bytes(),
        salt: salt.into_bytes(),
        k,
        m,
        response: tx,
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let manifest = rx.await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(manifest))
}

async fn download_file(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
    Query(query): Query<DownloadQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (tx, rx) = oneshot::channel();
    state.network_tx.send(Command::Download {
        file_id,
        passphrase: query.passphrase.into_bytes(),
        salt: query.salt.into_bytes(),
        k: query.k,
        m: query.m,
        response: tx,
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let file_data = rx.await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(file_data)
}
