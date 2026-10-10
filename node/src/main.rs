use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{Level, error, info};
use tracing_subscriber::FmtSubscriber;

use mesh_node::api::{AppState, make_router};
use mesh_node::identity::load_or_create_keypair;
use mesh_node::network::{Command, NetworkService};
use mesh_node::state::NodeState;

struct Args {
    p2p_port: u16,
    api_port: u16,
    api_bind: String,
    quota: f64,
    dial_peer: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        p2p_port: 4001,
        api_port: 3000,
        api_bind: env::var("MESH_API_BIND").unwrap_or_else(|_| "127.0.0.1".to_string()),
        quota: 1.5,
        dial_peer: None,
    };
    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-p" | "--port" => {
                if let Some(val) = iter.next()
                    && let Ok(p) = val.parse()
                {
                    args.p2p_port = p;
                }
            }
            "-a" | "--api-port" => {
                if let Some(val) = iter.next()
                    && let Ok(p) = val.parse()
                {
                    args.api_port = p;
                }
            }
            "-b" | "--bind" => {
                if let Some(val) = iter.next() {
                    args.api_bind = val;
                }
            }
            "-q" | "--quota" => {
                if let Some(val) = iter.next()
                    && let Ok(q) = val.parse()
                {
                    args.quota = q;
                }
            }
            "-d" | "--dial" => {
                if let Some(val) = iter.next() {
                    args.dial_peer = Some(val);
                }
            }
            _ => {}
        }
    }
    args
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // 2. Parse command-line args
    let args = parse_args();
    info!(
        "Starting node agent: p2p-port={}, api-port={}, quota={} GB",
        args.p2p_port, args.api_port, args.quota
    );

    // 3. Setup data directory and load/create keypair
    let data_dir = PathBuf::from(format!("./data_{}", args.p2p_port));
    fs::create_dir_all(&data_dir)?;
    let keypair = load_or_create_keypair(&data_dir)?;
    let peer_id = libp2p::PeerId::from(keypair.public());

    // 4. Initialize shared state
    let state = Arc::new(RwLock::new(NodeState::new(
        peer_id,
        args.p2p_port,
        args.quota,
    )));

    // 5. Initialize communication channels
    let (command_tx, command_rx) = mpsc::channel::<Command>(100);

    // 6. Initialize libp2p network service
    let network_service = NetworkService::new(keypair, state.clone(), command_rx)?;

    // 7. Setup and start HTTP API server
    let app_state = AppState {
        node_state: state.clone(),
        network_tx: command_tx.clone(),
    };
    let app = make_router(app_state);

    // Security guard: If binding outside loopback, enforce authentication
    let is_loopback = args.api_bind == "127.0.0.1" || args.api_bind == "localhost" || args.api_bind == "::1";
    if !is_loopback {
        let existing_key = env::var("MESH_API_KEY").unwrap_or_default();
        if existing_key.is_empty() {
            let generated_key = format!("mesh_sec_{}", hex::encode(rand::random::<[u8; 16]>()));
            tracing::warn!("═══════════════════════════════════════════════════════════════════");
            tracing::warn!("SECURITY ALERT: API bound to non-loopback interface ({}).", args.api_bind);
            tracing::warn!("Generated ephemeral API key: {}", generated_key);
            tracing::warn!("Set MESH_API_KEY environment variable or pass X-Mesh-Api-Key header.");
            tracing::warn!("═══════════════════════════════════════════════════════════════════");
            unsafe {
                env::set_var("MESH_API_KEY", &generated_key);
                env::set_var("MESH_REQUIRE_AUTH", "1");
            }
        }
    }

    let api_addr = format!("{}:{}", args.api_bind, args.api_port);
    info!("Starting HTTP API server on {}", api_addr);
    let listener = tokio::net::TcpListener::bind(&api_addr).await?;

    // Spawn API task
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("HTTP API server error: {}", e);
        }
    });

    // Dial peer if specified
    if let Some(peer_addr) = args.dial_peer {
        info!("Auto-dialing bootstrapper: {}", peer_addr);
        let _ = command_tx
            .send(Command::Pair {
                multiaddr: peer_addr,
                response: oneshot::channel().0, // Discard oneshot receiver
            })
            .await;
    }

    // 8. Run P2P network service (blocks main thread)
    network_service
        .run(args.p2p_port, command_tx.clone())
        .await?;

    Ok(())
}
