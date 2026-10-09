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
    quota: f64,
    dial_peer: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args {
        p2p_port: 4001,
        api_port: 3000,
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

    let api_addr = format!("0.0.0.0:{}", args.api_port);
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
