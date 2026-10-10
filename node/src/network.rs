use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::{
    Multiaddr, PeerId, Swarm, autonat, gossipsub, identify, kad, mdns, noise, ping, relay,
    request_response::{self, Codec, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{error, info, warn};

use crate::state::NodeState;
use mesh_core::{
    FileManifest, decode_file, encode_file,
    merkle::{Hash256, hash_data},
};

// Custom Request/Response protocol for shard transfer and audits
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ShardRequest {
    Store {
        file_id: String,
        chunk_idx: usize,
        shard_idx: usize,
        shard_hash: Hash256,
        data: Vec<u8>,
    },
    Retrieve {
        shard_hash: Hash256,
    },
    AuditChallenge {
        shard_hash: Hash256,
        nonce: [u8; 16],
    },
    Pair {
        caller_multiaddr: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ShardResponse {
    StoreAck { success: bool },
    RetrieveAck { data: Option<Vec<u8>> },
    AuditResponse { hash: Hash256 },
    PairAck { success: bool },
    Error(String),
}

pub const MAX_MESSAGE_SIZE: u64 = 16 * 1024 * 1024; // 16 MB maximum message limit

#[derive(Clone, Default)]
pub struct JsonCodec;

#[async_trait]
impl Codec for JsonCodec {
    type Protocol = &'static str;
    type Request = ShardRequest;
    type Response = ShardResponse;

    async fn read_request<T>(
        &mut self,
        _protocol: &&'static str,
        io: &mut T,
    ) -> std::io::Result<ShardRequest>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut vec = Vec::new();
        let mut limited = io.take(MAX_MESSAGE_SIZE);
        limited.read_to_end(&mut vec).await?;
        if vec.len() as u64 >= MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Request exceeded maximum message limit of 16MB",
            ));
        }
        serde_json::from_slice(&vec)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &&'static str,
        io: &mut T,
    ) -> std::io::Result<ShardResponse>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut vec = Vec::new();
        let mut limited = io.take(MAX_MESSAGE_SIZE);
        limited.read_to_end(&mut vec).await?;
        if vec.len() as u64 >= MAX_MESSAGE_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Response exceeded maximum message limit of 16MB",
            ));
        }
        serde_json::from_slice(&vec)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &&'static str,
        io: &mut T,
        req: ShardRequest,
    ) -> std::io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let bytes = serde_json::to_vec(&req)?;
        io.write_all(&bytes).await?;
        io.close().await?;
        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &&'static str,
        io: &mut T,
        res: ShardResponse,
    ) -> std::io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let bytes = serde_json::to_vec(&res)?;
        io.write_all(&bytes).await?;
        io.close().await?;
        Ok(())
    }
}

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "MyBehaviourEvent")]
pub struct MyBehaviour {
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
    pub request_response: request_response::Behaviour<JsonCodec>,
    pub relay: relay::Behaviour,
    pub autonat: autonat::Behaviour,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum MyBehaviourEvent {
    Kademlia(kad::Event),
    Mdns(mdns::Event),
    Gossipsub(gossipsub::Event),
    Ping(ping::Event),
    Identify(identify::Event),
    RequestResponse(request_response::Event<ShardRequest, ShardResponse>),
    Relay(relay::Event),
    Autonat(autonat::Event),
}

impl From<kad::Event> for MyBehaviourEvent {
    fn from(event: kad::Event) -> Self {
        MyBehaviourEvent::Kademlia(event)
    }
}

impl From<mdns::Event> for MyBehaviourEvent {
    fn from(event: mdns::Event) -> Self {
        MyBehaviourEvent::Mdns(event)
    }
}

impl From<gossipsub::Event> for MyBehaviourEvent {
    fn from(event: gossipsub::Event) -> Self {
        MyBehaviourEvent::Gossipsub(event)
    }
}

impl From<ping::Event> for MyBehaviourEvent {
    fn from(event: ping::Event) -> Self {
        MyBehaviourEvent::Ping(event)
    }
}

impl From<identify::Event> for MyBehaviourEvent {
    fn from(event: identify::Event) -> Self {
        MyBehaviourEvent::Identify(event)
    }
}

impl From<request_response::Event<ShardRequest, ShardResponse>> for MyBehaviourEvent {
    fn from(event: request_response::Event<ShardRequest, ShardResponse>) -> Self {
        MyBehaviourEvent::RequestResponse(event)
    }
}

impl From<relay::Event> for MyBehaviourEvent {
    fn from(event: relay::Event) -> Self {
        MyBehaviourEvent::Relay(event)
    }
}

impl From<autonat::Event> for MyBehaviourEvent {
    fn from(event: autonat::Event) -> Self {
        MyBehaviourEvent::Autonat(event)
    }
}

pub enum Command {
    Upload {
        file_id: String,
        temp_file_path: Option<std::path::PathBuf>,
        data: Option<Vec<u8>>,
        passphrase: Vec<u8>,
        salt: Vec<u8>,
        k: usize,
        m: usize,
        response: oneshot::Sender<Result<FileManifest>>,
    },
    Download {
        file_id: String,
        passphrase: Vec<u8>,
        salt: Vec<u8>,
        k: usize,
        m: usize,
        response: oneshot::Sender<Result<Vec<u8>>>,
    },
    Pair {
        multiaddr: String,
        response: oneshot::Sender<Result<()>>,
    },
    SendRequest {
        peer_id: PeerId,
        request: ShardRequest,
        response: oneshot::Sender<Result<ShardResponse>>,
    },
    AddPeerAddress {
        peer_id: PeerId,
        addr: Multiaddr,
    },
    GossipBroadcast {
        topic: String,
        data: Vec<u8>,
    },
}

fn get_ip_from_multiaddr(addr: &Multiaddr) -> Option<String> {
    for protocol in addr.iter() {
        match protocol {
            libp2p::multiaddr::Protocol::Ip4(ip) => return Some(ip.to_string()),
            libp2p::multiaddr::Protocol::Ip6(ip) => return Some(ip.to_string()),
            _ => {}
        }
    }
    None
}

/// Optional administrator feature to sync blocked IPs with OS-level firewalls.
/// Primary node defense is always application-layer libp2p identity verification,
/// signed enrollment tokens, peer revocation, and in-memory rate limiting.
fn block_ip_firewall(ip: &str) {
    let os_firewall_enabled = std::env::var("MESH_ENABLE_OS_FIREWALL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !os_firewall_enabled {
        info!(
            "Application-layer rate limit ban enforced for IP: {}. (OS-level firewall integration disabled by default)",
            ip
        );
        return;
    }

    #[cfg(target_os = "windows")]
    {
        info!(
            "MESH_ENABLE_OS_FIREWALL enabled: Triggering Windows Firewall block for IP: {}",
            ip
        );
        let rule_name = format!("MeshStorage Block {}", ip);
        let output = std::process::Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={}", rule_name),
                "dir=in",
                "action=block",
                &format!("remoteip={}", ip),
            ])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    info!(
                        "Successfully added Windows Firewall block rule for IP: {}",
                        ip
                    );
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr);
                    warn!(
                        "Windows Firewall netsh execution failed (likely requires admin): {}",
                        err_msg
                    );
                }
            }
            Err(e) => {
                warn!("Failed to invoke netsh: {}", e);
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        info!(
            "MESH_ENABLE_OS_FIREWALL enabled: Triggering Linux iptables block for IP: {} (requires root)",
            ip
        );
        let output = std::process::Command::new("iptables")
            .args(&["-A", "INPUT", "-s", ip, "-j", "DROP"])
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    info!("Successfully added iptables block rule for IP: {}", ip);
                } else {
                    let err_msg = String::from_utf8_lossy(&out.stderr);
                    warn!(
                        "Linux iptables execution failed (likely requires root/sudo): {}",
                        err_msg.trim()
                    );
                }
            }
            Err(e) => {
                warn!("Failed to invoke iptables: {}", e);
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "android")))]
    {
        info!(
            "OS-level firewall blocking not implemented for OS ({}). Application-layer in-memory blocking remains active.",
            std::env::consts::OS
        );
    }
}

pub struct NetworkService {
    swarm: Swarm<MyBehaviour>,
    state: Arc<RwLock<NodeState>>,
    command_rx: mpsc::Receiver<Command>,
    pending_requests:
        HashMap<request_response::OutboundRequestId, oneshot::Sender<Result<ShardResponse>>>,
    unlisted_attempts: HashMap<String, (usize, std::time::Instant)>,
    blocked_ips: HashMap<String, std::time::Instant>,
}

impl NetworkService {
    pub fn new(
        keypair: libp2p::identity::Keypair,
        state: Arc<RwLock<NodeState>>,
        command_rx: mpsc::Receiver<Command>,
    ) -> Result<Self> {
        let local_peer_id = PeerId::from(keypair.public());
        info!("Local Peer ID: {}", local_peer_id);

        let store = kad::store::MemoryStore::new(local_peer_id);
        let kademlia = kad::Behaviour::new(local_peer_id, store);

        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .build()
            .map_err(|e| anyhow!("Gossipsub config error: {:?}", e))?;
        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(|e| anyhow!("Gossipsub init error: {:?}", e))?;

        let ping = ping::Behaviour::new(ping::Config::default());

        let identify = identify::Behaviour::new(identify::Config::new(
            "/mesh-storage/1.0.0".to_string(),
            keypair.public(),
        ));

        let request_response = request_response::Behaviour::new(
            std::iter::once((
                "/mesh-storage/request-response/1.0.0",
                ProtocolSupport::Full,
            )),
            request_response::Config::default(),
        );

        let relay = relay::Behaviour::new(local_peer_id, relay::Config::default());
        let autonat = autonat::Behaviour::new(local_peer_id, autonat::Config::default());

        let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|_key| MyBehaviour {
                kademlia,
                mdns,
                gossipsub,
                ping,
                identify,
                request_response,
                relay,
                autonat,
            })?
            .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        Ok(Self {
            swarm,
            state,
            command_rx,
            pending_requests: HashMap::new(),
            unlisted_attempts: HashMap::new(),
            blocked_ips: HashMap::new(),
        })
    }

    pub async fn run(mut self, port: u16, command_tx: mpsc::Sender<Command>) -> Result<()> {
        let addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", port).parse()?;
        self.swarm.listen_on(addr)?;

        // Subscribe to gossipsub topic for network events
        let topic = gossipsub::IdentTopic::new("mesh-events");
        self.swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    if let Err(e) = self.handle_swarm_event(event).await {
                        error!("Error handling swarm event: {}", e);
                    }
                }
                cmd = self.command_rx.recv() => {
                    if let Some(cmd) = cmd {
                        self.handle_command(cmd, command_tx.clone()).await;
                    }
                }
            }
        }
    }

    async fn handle_swarm_event(&mut self, event: SwarmEvent<MyBehaviourEvent>) -> Result<()> {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Swarm listening on address: {}", address);
                let mut state = self.state.write().await;
                state.listen_addresses.insert(address);
            }
            SwarmEvent::IncomingConnection { send_back_addr, .. } => {
                if let Some(ip) = get_ip_from_multiaddr(&send_back_addr)
                    && let Some(expiry) = self.blocked_ips.get(&ip)
                    && std::time::Instant::now() < *expiry
                {
                    warn!(
                        "Incoming connection from explicitly blocked IP: {}. Connection will be rejected.",
                        ip
                    );
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                let mut state = self.state.write().await;

                let remote_ip_opt = get_ip_from_multiaddr(endpoint.get_remote_address());
                if let Some(ref remote_ip) = remote_ip_opt
                    && let Some(expiry) = self.blocked_ips.get(remote_ip)
                    && std::time::Instant::now() < *expiry
                {
                    warn!(
                        "Disconnecting connection from explicitly blocked IP: {}",
                        remote_ip
                    );
                    self.swarm.disconnect_peer_id(peer_id).unwrap_or_default();
                    return Ok(());
                }

                if state.is_trusted(&peer_id) {
                    info!("Connection established with trusted PeerID: {}", peer_id);
                    state.connected_peers.insert(peer_id);
                    // Add to Kademlia routing
                    self.swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, endpoint.get_remote_address().clone());
                } else {
                    info!(
                        "Connection established with unauthenticated PeerID: {} (awaiting handshake / pair request)",
                        peer_id
                    );
                }
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                info!("Connection closed with PeerID: {}", peer_id);
                let mut state = self.state.write().await;
                state.connected_peers.remove(&peer_id);
            }
            SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                for (peer_id, addr) in peers {
                    let state = self.state.read().await;
                    if state.is_trusted(&peer_id) {
                        info!("mDNS discovered trusted peer {} at {}", peer_id, addr);
                        self.swarm
                            .behaviour_mut()
                            .kademlia
                            .add_address(&peer_id, addr);
                    }
                }
            }
            SwarmEvent::Behaviour(MyBehaviourEvent::RequestResponse(
                request_response::Event::Message { peer, message },
            )) => {
                self.handle_request_response(peer, message).await?;
            }
            SwarmEvent::Behaviour(MyBehaviourEvent::RequestResponse(
                request_response::Event::OutboundFailure {
                    request_id, error, ..
                },
            )) => {
                warn!(
                    "Outbound request-response failure {:?}: {:?}",
                    request_id, error
                );
                if let Some(tx) = self.pending_requests.remove(&request_id) {
                    tx.send(Err(anyhow!("Outbound request failure: {:?}", error)))
                        .unwrap_or_default();
                }
            }
            SwarmEvent::Behaviour(MyBehaviourEvent::RequestResponse(
                request_response::Event::ResponseSent { .. },
            )) => {}
            SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                ..
            })) => {
                if let Ok(event_str) = String::from_utf8(message.data) {
                    info!("Gossipsub Event Received: {}", event_str);
                }
            }
            SwarmEvent::Behaviour(MyBehaviourEvent::Autonat(autonat::Event::StatusChanged {
                new,
                ..
            })) => {
                let mut state = self.state.write().await;
                let status_str = match new {
                    autonat::NatStatus::Public(addr) => format!("Public ({})", addr),
                    autonat::NatStatus::Private => "Private (behind NAT)".to_string(),
                    autonat::NatStatus::Unknown => "Unknown".to_string(),
                };
                info!("AutoNAT status updated: {}", status_str);
                state.set_nat_status(status_str);
            }
            SwarmEvent::Behaviour(MyBehaviourEvent::Relay(relay_event)) => match relay_event {
                relay::Event::ReservationReqAccepted { src_peer_id, .. } => {
                    info!("Relay v2 reservation accepted for peer: {}", src_peer_id);
                }
                relay::Event::CircuitReqAccepted {
                    src_peer_id,
                    dst_peer_id,
                } => {
                    info!(
                        "Relay v2 circuit accepted between {} and {}",
                        src_peer_id, dst_peer_id
                    );
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    async fn handle_request_response(
        &mut self,
        peer_id: PeerId,
        message: request_response::Message<ShardRequest, ShardResponse>,
    ) -> Result<()> {
        match message {
            request_response::Message::Request {
                channel, request, ..
            } => {
                // Pre-auth connection check
                let mut state = self.state.write().await;
                if !state.is_trusted(&peer_id) {
                    // Check if it's a Pair Request
                    if let ShardRequest::Pair {
                        ref caller_multiaddr,
                    } = request
                    {
                        info!(
                            "Received pairing request from peer {} with multiaddr {}",
                            peer_id, caller_multiaddr
                        );
                        state.add_trusted_peer(peer_id);
                        state.connected_peers.insert(peer_id);
                        if let Ok(addr) = caller_multiaddr.parse::<Multiaddr>() {
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, addr);
                        }
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, ShardResponse::PairAck { success: true })
                            .unwrap_or_default();
                        return Ok(());
                    }
                    warn!("Rejecting request from untrusted peer {}", peer_id);
                    let now = std::time::Instant::now();
                    let peer_str = peer_id.to_string();
                    let attempt = self
                        .unlisted_attempts
                        .entry(peer_str.clone())
                        .or_insert((0, now));
                    if now.duration_since(attempt.1) < std::time::Duration::from_secs(60) {
                        attempt.0 += 1;
                        if attempt.0 >= 5 {
                            warn!(
                                "Peer {} exceeded 5 untrusted requests in 60s. Enforcing ban.",
                                peer_str
                            );
                            block_ip_firewall(&peer_str);
                        }
                    } else {
                        *attempt = (1, now);
                    }

                    self.swarm
                        .behaviour_mut()
                        .request_response
                        .send_response(channel, ShardResponse::Error("Untrusted peer".to_string()))
                        .unwrap_or_default();
                    self.swarm.disconnect_peer_id(peer_id).unwrap_or_default();
                    return Ok(());
                }

                // Handle authenticated requests
                match request {
                    ShardRequest::Store {
                        shard_hash, data, ..
                    } => {
                        let data_len = data.len() as u64;
                        if !state.check_egress_bandwidth(data_len) {
                            warn!("Throttling store request: bandwidth limit exceeded");
                            self.swarm
                                .behaviour_mut()
                                .request_response
                                .send_response(
                                    channel,
                                    ShardResponse::Error("Bandwidth limit exceeded".to_string()),
                                )
                                .unwrap_or_default();
                            return Ok(());
                        }

                        let hash_hex = hex::encode(shard_hash);
                        match state.write_shard(&hash_hex, &data) {
                            Ok(_) => {
                                info!("Stored shard {} successfully", hash_hex);
                                self.swarm
                                    .behaviour_mut()
                                    .request_response
                                    .send_response(
                                        channel,
                                        ShardResponse::StoreAck { success: true },
                                    )
                                    .unwrap_or_default();
                            }
                            Err(e) => {
                                error!("Failed to store shard {}: {}", hash_hex, e);
                                self.swarm
                                    .behaviour_mut()
                                    .request_response
                                    .send_response(channel, ShardResponse::Error(e))
                                    .unwrap_or_default();
                            }
                        }
                    }
                    ShardRequest::Retrieve { shard_hash } => {
                        let hash_hex = hex::encode(shard_hash);
                        let data = state.read_shard(&hash_hex);
                        if let Some(ref bytes) = data
                            && !state.check_egress_bandwidth(bytes.len() as u64)
                        {
                            warn!("Throttling retrieve request: bandwidth limit exceeded");
                            self.swarm
                                .behaviour_mut()
                                .request_response
                                .send_response(
                                    channel,
                                    ShardResponse::Error("Bandwidth limit exceeded".to_string()),
                                )
                                .unwrap_or_default();
                            return Ok(());
                        }
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, ShardResponse::RetrieveAck { data })
                            .unwrap_or_default();
                    }
                    ShardRequest::AuditChallenge { shard_hash, nonce } => {
                        let hash_hex = hex::encode(shard_hash);
                        if let Some(bytes) = state.read_shard(&hash_hex) {
                            let mut hash_input = bytes.clone();
                            hash_input.extend_from_slice(&nonce);
                            let audit_hash = hash_data(&hash_input);
                            self.swarm
                                .behaviour_mut()
                                .request_response
                                .send_response(
                                    channel,
                                    ShardResponse::AuditResponse { hash: audit_hash },
                                )
                                .unwrap_or_default();
                        } else {
                            self.swarm
                                .behaviour_mut()
                                .request_response
                                .send_response(
                                    channel,
                                    ShardResponse::Error("Shard not found".to_string()),
                                )
                                .unwrap_or_default();
                        }
                    }
                    ShardRequest::Pair { .. } => {
                        // Already handled above
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, ShardResponse::PairAck { success: true })
                            .unwrap_or_default();
                    }
                }
            }
            request_response::Message::Response {
                request_id,
                response,
            } => {
                if let Some(tx) = self.pending_requests.remove(&request_id) {
                    tx.send(Ok(response)).unwrap_or_default();
                }
            }
        }
        Ok(())
    }

    async fn handle_command(&mut self, cmd: Command, self_tx: mpsc::Sender<Command>) {
        match cmd {
            Command::Upload {
                file_id,
                temp_file_path,
                data,
                passphrase,
                salt,
                k,
                m,
                response,
            } => {
                let state = self.state.clone();
                let swarm_local_peer_id = *self.swarm.local_peer_id();
                tokio::spawn(async move {
                    let res = perform_upload_async(
                        file_id,
                        temp_file_path,
                        data,
                        passphrase,
                        salt,
                        k,
                        m,
                        state,
                        swarm_local_peer_id,
                        self_tx,
                    )
                    .await;
                    response.send(res).unwrap_or_default();
                });
            }
            Command::Download {
                file_id,
                passphrase,
                salt,
                k,
                m,
                response,
            } => {
                let state = self.state.clone();
                let swarm_local_peer_id = *self.swarm.local_peer_id();
                tokio::spawn(async move {
                    let res = perform_download_async(
                        file_id,
                        passphrase,
                        salt,
                        k,
                        m,
                        state,
                        swarm_local_peer_id,
                        self_tx,
                    )
                    .await;
                    response.send(res).unwrap_or_default();
                });
            }
            Command::Pair {
                multiaddr,
                response,
            } => {
                let state = self.state.clone();
                let self_tx_clone = self_tx.clone();
                tokio::spawn(async move {
                    let res = perform_pair_async(multiaddr, state, self_tx_clone).await;
                    response.send(res).unwrap_or_default();
                });
            }
            Command::SendRequest {
                peer_id,
                request,
                response,
            } => {
                let req_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer_id, request);
                self.pending_requests.insert(req_id, response);
            }
            Command::AddPeerAddress { peer_id, addr } => {
                let _ = self.swarm.dial(addr.clone());
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr);
            }
            Command::GossipBroadcast { topic, data } => {
                let gossip_topic = gossipsub::IdentTopic::new(topic);
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(gossip_topic, data);
            }
        }
    }
}

async fn perform_pair_async(
    multiaddr_str: String,
    state: Arc<RwLock<NodeState>>,
    self_tx: mpsc::Sender<Command>,
) -> Result<()> {
    let addr: Multiaddr = multiaddr_str.parse()?;

    // Extract PeerId from multiaddr
    let mut peer_id_opt = None;
    for protocol in addr.iter() {
        if let libp2p::multiaddr::Protocol::P2p(p) = protocol {
            peer_id_opt = Some(p);
            break;
        }
    }
    let peer_id =
        peer_id_opt.ok_or_else(|| anyhow!("Multiaddr does not contain p2p PeerID component"))?;

    // Add to state trusted list first to allow connection
    {
        let mut s = state.write().await;
        s.add_trusted_peer(peer_id);
    }

    // Command the Swarm to add address and dial
    self_tx
        .send(Command::AddPeerAddress { peer_id, addr })
        .await
        .map_err(|e| anyhow!("Failed to send AddPeerAddress command: {}", e))?;

    // Wait until connection is established (up to 3 seconds)
    for _ in 0..30 {
        {
            let s = state.read().await;
            if s.connected_peers.contains(&peer_id) {
                break;
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Send a Pair Request
    let local_addr = {
        let s = state.read().await;
        s.listen_addresses
            .iter()
            .next()
            .map(|a| a.to_string())
            .unwrap_or_default()
    };

    let (tx, rx) = oneshot::channel();
    self_tx
        .send(Command::SendRequest {
            peer_id,
            request: ShardRequest::Pair {
                caller_multiaddr: local_addr,
            },
            response: tx,
        })
        .await
        .map_err(|e| anyhow!("Failed to send SendRequest command: {}", e))?;

    match rx.await? {
        Ok(ShardResponse::PairAck { success: true }) => {
            info!("Pairing with peer {} succeeded", peer_id);
            let mut s = state.write().await;
            s.connected_peers.insert(peer_id);
            Ok(())
        }
        other => Err(anyhow!("Pairing failed: {:?}", other)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn perform_upload_async(
    file_id: String,
    temp_file_path: Option<std::path::PathBuf>,
    data: Option<Vec<u8>>,
    passphrase: Vec<u8>,
    salt: Vec<u8>,
    k: usize,
    m: usize,
    state: Arc<RwLock<NodeState>>,
    swarm_local_peer_id: PeerId,
    self_tx: mpsc::Sender<Command>,
) -> Result<FileManifest> {
    info!("Starting upload for file ID: {}", file_id);
    // 1. Encode file in core library (uses streaming encode_reader if temp_file_path provided)
    let (manifest, encoded_chunks) = if let Some(ref path) = temp_file_path {
        let f = std::fs::File::open(path)
            .map_err(|e| anyhow!("Failed to open temp upload file: {}", e))?;
        let res = mesh_core::encode_reader(f, &passphrase, &salt, &file_id, k, m)
            .map_err(|e| anyhow!("Core encode reader error: {}", e))?;
        let _ = std::fs::remove_file(path);
        res
    } else {
        let raw = data.unwrap_or_default();
        encode_file(&raw, &passphrase, &salt, &file_id, k, m)
            .map_err(|e| anyhow!("Core encode error: {}", e))?
    };

    // 2. Select storage nodes for each shard of each chunk.
    let peers = {
        let s = state.read().await;
        let mut list = Vec::new();
        list.push(s.peer_id); // Include ourselves
        for peer in &s.connected_peers {
            list.push(*peer);
        }
        list
    };

    if peers.len() < k + m {
        return Err(anyhow!(
            "Insufficient nodes: have {} nodes, but configuration needs at least {}",
            peers.len(),
            k + m
        ));
    }

    // For each chunk, send its shards to selected peers
    for (chunk_idx, chunk_shards) in encoded_chunks.into_iter().enumerate() {
        let chunk_peers = &peers[0..(k + m)];
        let chunk_manifest = &manifest.chunks[chunk_idx];

        for (shard_idx, shard_data) in chunk_shards.into_iter().enumerate() {
            let peer = &chunk_peers[shard_idx];
            let shard_hash = chunk_manifest.shard_hashes[shard_idx];

            if peer == &swarm_local_peer_id {
                // Store locally
                let mut s = state.write().await;
                s.write_shard(&hex::encode(shard_hash), &shard_data)
                    .map_err(|e| anyhow!("Local write error: {}", e))?;
            } else {
                // Send request to store on peer
                let (tx, rx) = oneshot::channel();
                self_tx
                    .send(Command::SendRequest {
                        peer_id: *peer,
                        request: ShardRequest::Store {
                            file_id: file_id.clone(),
                            chunk_idx,
                            shard_idx,
                            shard_hash,
                            data: shard_data,
                        },
                        response: tx,
                    })
                    .await
                    .map_err(|e| anyhow!("Failed to send Store request: {}", e))?;

                match rx.await? {
                    Ok(ShardResponse::StoreAck { success: true }) => {
                        info!(
                            "Peer {} acknowledged store of shard {}",
                            peer,
                            hex::encode(shard_hash)
                        );
                    }
                    other => {
                        return Err(anyhow!("Peer failed to store shard: {:?}", other));
                    }
                }
            }
        }
    }

    // Save manifest locally
    {
        let s = state.read().await;
        s.save_manifest(&manifest).map_err(|e| anyhow!(e))?;
    }

    // Gossip about the file upload
    let gossip_msg = serde_json::json!({
        "type": "upload_complete",
        "file_id": file_id,
        "root_hash": manifest.root_hash,
    });
    if let Ok(msg_bytes) = serde_json::to_vec(&gossip_msg) {
        let _ = self_tx
            .send(Command::GossipBroadcast {
                topic: "mesh-events".to_string(),
                data: msg_bytes,
            })
            .await;
    }

    Ok(manifest)
}

#[allow(clippy::too_many_arguments)]
async fn perform_download_async(
    file_id: String,
    passphrase: Vec<u8>,
    salt: Vec<u8>,
    k: usize,
    m: usize,
    state: Arc<RwLock<NodeState>>,
    swarm_local_peer_id: PeerId,
    self_tx: mpsc::Sender<Command>,
) -> Result<Vec<u8>> {
    info!("Starting download for file ID: {}", file_id);

    // Read manifest locally
    let manifest = {
        let s = state.read().await;
        s.read_manifest(&file_id)
            .ok_or_else(|| anyhow!("Manifest for file {} not found locally", file_id))?
    };

    let peers = {
        let s = state.read().await;
        let mut list = Vec::new();
        list.push(s.peer_id);
        for peer in &s.connected_peers {
            list.push(*peer);
        }
        list
    };

    let mut retrieved_shards = Vec::new();

    for chunk_manifest in manifest.chunks.iter() {
        let mut chunk_provided_shards = Vec::new();
        let chunk_peers = &peers[0..(k + m)];

        for (shard_idx, shard_hash) in chunk_manifest.shard_hashes.iter().enumerate() {
            let peer = &chunk_peers[shard_idx];

            if peer == &swarm_local_peer_id {
                // Read locally
                let s = state.read().await;
                let local_data = s.read_shard(&hex::encode(shard_hash));
                chunk_provided_shards.push(local_data);
            } else {
                // Request from peer
                let (tx, rx) = oneshot::channel();
                self_tx
                    .send(Command::SendRequest {
                        peer_id: *peer,
                        request: ShardRequest::Retrieve {
                            shard_hash: *shard_hash,
                        },
                        response: tx,
                    })
                    .await
                    .map_err(|e| anyhow!("Failed to send Retrieve request: {}", e))?;

                match rx.await {
                    Ok(Ok(ShardResponse::RetrieveAck { data: Some(bytes) })) => {
                        chunk_provided_shards.push(Some(bytes));
                    }
                    other => {
                        warn!(
                            "Failed to retrieve shard {} from peer {}: {:?}",
                            hex::encode(shard_hash),
                            peer,
                            other
                        );
                        chunk_provided_shards.push(None);
                    }
                }
            }
        }

        retrieved_shards.push(chunk_provided_shards);
    }

    // Call core decode
    let plaintext_data = decode_file(&manifest, &passphrase, &salt, &retrieved_shards, k, m)
        .map_err(|e| anyhow!("Core decode error: {}", e))?;

    // Gossip about the file download
    let gossip_msg = serde_json::json!({
        "type": "download_complete",
        "file_id": file_id,
    });
    if let Ok(msg_bytes) = serde_json::to_vec(&gossip_msg) {
        let _ = self_tx
            .send(Command::GossipBroadcast {
                topic: "mesh-events".to_string(),
                data: msg_bytes,
            })
            .await;
    }

    Ok(plaintext_data)
}
