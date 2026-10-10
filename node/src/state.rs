use libp2p::{Multiaddr, PeerId};
use mesh_core::FileManifest;
use mesh_core::quota::{QuotaPolicy, QuotaTracker};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use tracing::{error, info};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeLifecycleState {
    #[default]
    Active,
    Paused,
    Leaving,
    Revoked,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NodeStatus {
    pub peer_id: String,
    pub state: NodeLifecycleState,
    pub listen_addresses: Vec<String>,
    pub peers: Vec<String>,
    pub storage_used: u64,
    pub storage_quota: u64,
    pub usage_ratio: f64,
    pub remaining_bytes: u64,
    pub shards: Vec<String>,
    pub trusted_peers: Vec<String>,
    pub nat_status: String,
    pub relay_addresses: Vec<String>,
    pub bandwidth_limit_kbps: Option<u64>,
}

pub struct NodeState {
    pub peer_id: PeerId,
    pub state: NodeLifecycleState,
    pub listen_addresses: HashSet<Multiaddr>,
    pub connected_peers: HashSet<PeerId>,
    pub trusted_peers: HashSet<PeerId>,
    pub consumed_nonces: HashSet<String>,
    pub quota_tracker: QuotaTracker,
    pub storage_quota: u64,
    pub storage_used: u64,
    pub data_dir: PathBuf,
    pub nat_status: String,
    pub relay_addresses: HashSet<Multiaddr>,
    pub bandwidth_limiter: Option<mesh_core::BandwidthLimiter>,
    pub peer_reliability: HashMap<String, mesh_core::PeerReliabilityTracker>,
    pub local_credits: mesh_core::CreditLedger,
    pub peer_credits: HashMap<String, mesh_core::CreditLedger>,
    pub subnet_guard: mesh_core::SubnetDensityGuard,
}

#[allow(dead_code)]
impl NodeState {
    pub fn new(peer_id: PeerId, port: u16, quota_gb: f64) -> Self {
        let data_dir = PathBuf::from(format!("./data_{}", port));
        Self::with_data_dir(peer_id, data_dir, quota_gb)
    }

    pub fn with_data_dir(peer_id: PeerId, data_dir: PathBuf, quota_gb: f64) -> Self {
        fs::create_dir_all(data_dir.join("shards")).unwrap_or_default();
        fs::create_dir_all(data_dir.join("manifests")).unwrap_or_default();

        let quota_bytes = (quota_gb * 1024.0 * 1024.0 * 1024.0).round() as u64;
        let quota_tracker =
            QuotaTracker::from_policy(quota_bytes, &QuotaPolicy::AbsoluteBytes(quota_bytes))
                .unwrap_or_else(|_| QuotaTracker::new(quota_bytes, quota_bytes));

        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut state = Self {
            peer_id,
            state: NodeLifecycleState::Active,
            listen_addresses: HashSet::new(),
            connected_peers: HashSet::new(),
            trusted_peers: HashSet::new(),
            consumed_nonces: HashSet::new(),
            quota_tracker,
            storage_quota: quota_bytes,
            storage_used: 0,
            data_dir,
            nat_status: "Unknown".to_string(),
            relay_addresses: HashSet::new(),
            bandwidth_limiter: None,
            peer_reliability: HashMap::new(),
            local_credits: mesh_core::CreditLedger::new(peer_id.to_string(), now_sec),
            peer_credits: HashMap::new(),
            subnet_guard: mesh_core::SubnetDensityGuard::new(3),
        };

        state.load_trusted_peers();
        state.load_consumed_nonces();
        state.recalculate_storage_used();
        state
    }

    pub fn validate_hash_key(hash_hex: &str) -> Result<(), String> {
        if hash_hex.is_empty() || hash_hex.len() > 128 {
            return Err("Invalid hash length".to_string());
        }
        if !hash_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Hash contains non-hexadecimal characters".to_string());
        }
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if self.state == NodeLifecycleState::Revoked {
            return Err("Cannot pause a revoked node".to_string());
        }
        self.state = NodeLifecycleState::Paused;
        info!("Node {} state transitioned to Paused", self.peer_id);
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.state == NodeLifecycleState::Revoked {
            return Err("Cannot resume a revoked node".to_string());
        }
        self.state = NodeLifecycleState::Active;
        info!("Node {} state transitioned to Active", self.peer_id);
        Ok(())
    }

    pub fn leave(&mut self) -> Result<(), String> {
        self.state = NodeLifecycleState::Leaving;
        info!(
            "Node {} state transitioned to Leaving (initiating shard handoff)",
            self.peer_id
        );
        Ok(())
    }

    pub fn load_trusted_peers(&mut self) {
        let path = self.data_dir.join("trusted_peers.json");
        if path.exists()
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(list) = serde_json::from_str::<Vec<String>>(&content)
        {
            for peer_str in list {
                if let Ok(peer_id) = peer_str.parse::<PeerId>() {
                    self.trusted_peers.insert(peer_id);
                }
            }
        }
        // Always trust ourselves
        self.trusted_peers.insert(self.peer_id);
        info!("Loaded {} trusted peers", self.trusted_peers.len());
    }

    pub fn save_trusted_peers(&self) {
        let path = self.data_dir.join("trusted_peers.json");
        let list: Vec<String> = self.trusted_peers.iter().map(|p| p.to_string()).collect();
        if let Ok(content) = serde_json::to_string_pretty(&list)
            && let Err(e) = fs::write(&path, content)
        {
            error!("Failed to write trusted_peers.json: {}", e);
        }
    }

    pub fn add_trusted_peer(&mut self, peer_id: PeerId) {
        if self.trusted_peers.insert(peer_id) {
            info!("Added trusted peer: {}", peer_id);
            self.save_trusted_peers();
        }
    }

    pub fn is_trusted(&self, peer_id: &PeerId) -> bool {
        self.trusted_peers.contains(peer_id)
    }

    pub fn load_consumed_nonces(&mut self) {
        let path = self.data_dir.join("consumed_nonces.json");
        if path.exists()
            && let Ok(content) = fs::read_to_string(&path)
            && let Ok(list) = serde_json::from_str::<Vec<String>>(&content)
        {
            self.consumed_nonces = list.into_iter().collect();
            info!(
                "Loaded {} consumed invitation nonces",
                self.consumed_nonces.len()
            );
        }
    }

    pub fn save_consumed_nonces(&self) {
        let path = self.data_dir.join("consumed_nonces.json");
        let list: Vec<String> = self.consumed_nonces.iter().cloned().collect();
        if let Ok(content) = serde_json::to_string_pretty(&list)
            && let Err(e) = fs::write(&path, content)
        {
            error!("Failed to write consumed_nonces.json: {}", e);
        }
    }

    pub fn consume_nonce(&mut self, nonce: &str) -> Result<(), String> {
        if self.consumed_nonces.contains(nonce) {
            return Err("Invitation nonce has already been used".to_string());
        }
        self.consumed_nonces.insert(nonce.to_string());
        self.save_consumed_nonces();
        info!("Consumed invitation single-use nonce: {}", nonce);
        Ok(())
    }

    pub fn recalculate_storage_used(&mut self) {
        let mut total = 0u64;
        let shards_dir = self.data_dir.join("shards");
        if let Ok(entries) = fs::read_dir(shards_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata()
                    && metadata.is_file()
                {
                    total += metadata.len();
                }
            }
        }
        self.storage_used = total;
        self.quota_tracker.used_bytes = total;
        info!(
            "Storage used: {} / {} bytes",
            self.storage_used, self.storage_quota
        );
    }

    pub fn has_shard(&self, hash_hex: &str) -> bool {
        if Self::validate_hash_key(hash_hex).is_err() {
            return false;
        }
        self.data_dir.join("shards").join(hash_hex).exists()
    }

    pub fn read_shard(&self, hash_hex: &str) -> Option<Vec<u8>> {
        if Self::validate_hash_key(hash_hex).is_err() {
            return None;
        }
        let path = self.data_dir.join("shards").join(hash_hex);
        if path.exists() {
            fs::read(path).ok()
        } else {
            None
        }
    }

    /// Atomically writes a shard using a temporary file and rename.
    /// Rejects writes if the node is paused/leaving or if quota is exceeded.
    pub fn write_shard(&mut self, hash_hex: &str, data: &[u8]) -> Result<(), String> {
        if self.state == NodeLifecycleState::Paused {
            return Err("Node is currently paused".to_string());
        }
        if self.state == NodeLifecycleState::Leaving || self.state == NodeLifecycleState::Revoked {
            return Err("Node is leaving or revoked, rejecting store".to_string());
        }
        Self::validate_hash_key(hash_hex)?;

        let shard_len = data.len() as u64;
        let shards_dir = self.data_dir.join("shards");
        let dest_path = shards_dir.join(hash_hex);

        // Idempotent write if shard already exists
        if dest_path.exists() {
            return Ok(());
        }

        self.quota_tracker
            .record_store(shard_len)
            .map_err(|e| e.to_string())?;

        let tmp_name = format!(".tmp_{}_{}", hash_hex, rand::random::<u32>());
        let tmp_path = shards_dir.join(tmp_name);

        if let Err(e) = fs::write(&tmp_path, data) {
            self.quota_tracker.record_delete(shard_len);
            return Err(e.to_string());
        }

        if let Err(e) = fs::rename(&tmp_path, &dest_path) {
            let _ = fs::remove_file(&tmp_path);
            self.quota_tracker.record_delete(shard_len);
            return Err(e.to_string());
        }

        self.storage_used = self.quota_tracker.used_bytes;
        self.local_credits.record_storage_contribution(shard_len);
        Ok(())
    }

    pub fn delete_shard(&mut self, hash_hex: &str) -> Result<(), String> {
        Self::validate_hash_key(hash_hex)?;
        let path = self.data_dir.join("shards").join(hash_hex);
        if path.exists() {
            let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            fs::remove_file(path).map_err(|e| e.to_string())?;
            self.quota_tracker.record_delete(len);
            self.storage_used = self.quota_tracker.used_bytes;
        }
        Ok(())
    }

    pub fn set_quota(&mut self, quota_bytes: u64) {
        self.storage_quota = quota_bytes;
        self.quota_tracker.quota_bytes = quota_bytes;
        info!(
            "Node quota updated: {} bytes (used: {} bytes)",
            self.storage_quota, self.storage_used
        );
    }

    fn safe_manifest_filename(file_id: &str) -> String {
        let sanitized: String = file_id
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{}.json", sanitized)
    }

    pub fn save_manifest(&self, manifest: &FileManifest) -> Result<(), String> {
        let filename = Self::safe_manifest_filename(&manifest.file_id);
        let path = self.data_dir.join("manifests").join(filename);
        let content = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn read_manifest(&self, file_id: &str) -> Option<FileManifest> {
        let filename = Self::safe_manifest_filename(file_id);
        let path = self.data_dir.join("manifests").join(filename);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                serde_json::from_str(&content).ok()
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn list_manifests(&self) -> Vec<FileManifest> {
        let mut list = Vec::new();
        let dir = self.data_dir.join("manifests");
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Ok(content) = fs::read_to_string(entry.path())
                    && let Ok(manifest) = serde_json::from_str::<FileManifest>(&content)
                {
                    list.push(manifest);
                }
            }
        }
        list
    }

    pub fn set_nat_status(&mut self, status: String) {
        self.nat_status = status;
    }

    pub fn add_relay_address(&mut self, addr: Multiaddr) {
        self.relay_addresses.insert(addr);
    }

    pub fn set_bandwidth_limit(&mut self, kbps: Option<u64>) {
        if let Some(rate_kbps) = kbps {
            let bytes_per_sec = rate_kbps * 1024;
            let burst_capacity = bytes_per_sec * 2;
            self.bandwidth_limiter = Some(mesh_core::BandwidthLimiter::new(
                bytes_per_sec,
                burst_capacity,
            ));
        } else {
            self.bandwidth_limiter = None;
        }
    }

    pub fn check_egress_bandwidth(&mut self, bytes: u64) -> bool {
        if let Some(ref mut limiter) = self.bandwidth_limiter {
            limiter.try_acquire(bytes)
        } else {
            true
        }
    }

    pub fn get_status(&self) -> NodeStatus {
        let shards = fs::read_dir(self.data_dir.join("shards"))
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|name| !name.starts_with(".tmp"))
                    .collect()
            })
            .unwrap_or_default();

        let bandwidth_limit_kbps = self
            .bandwidth_limiter
            .as_ref()
            .map(|lim| lim.max_rate_bytes_per_sec / 1024);

        NodeStatus {
            peer_id: self.peer_id.to_string(),
            state: self.state,
            listen_addresses: self
                .listen_addresses
                .iter()
                .map(|a| a.to_string())
                .collect(),
            peers: self.connected_peers.iter().map(|p| p.to_string()).collect(),
            storage_used: self.storage_used,
            storage_quota: self.storage_quota,
            usage_ratio: self.quota_tracker.usage_ratio(),
            remaining_bytes: self.quota_tracker.remaining_bytes(),
            shards,
            trusted_peers: self.trusted_peers.iter().map(|p| p.to_string()).collect(),
            nat_status: self.nat_status.clone(),
            relay_addresses: self.relay_addresses.iter().map(|a| a.to_string()).collect(),
            bandwidth_limit_kbps,
        }
    }

    pub fn record_audit_success(&mut self, peer_id: &str, timestamp: u64) {
        let tracker = self
            .peer_reliability
            .entry(peer_id.to_string())
            .or_insert_with(|| mesh_core::PeerReliabilityTracker::new(peer_id.to_string()));
        tracker.record_success(timestamp);

        self.peer_credits
            .entry(peer_id.to_string())
            .or_insert_with(|| mesh_core::CreditLedger::new(peer_id.to_string(), timestamp))
            .record_audit(true);
    }

    pub fn record_audit_failure(&mut self, peer_id: &str, timestamp: u64) {
        let tracker = self
            .peer_reliability
            .entry(peer_id.to_string())
            .or_insert_with(|| mesh_core::PeerReliabilityTracker::new(peer_id.to_string()));
        tracker.record_failure(timestamp);

        self.peer_credits
            .entry(peer_id.to_string())
            .or_insert_with(|| mesh_core::CreditLedger::new(peer_id.to_string(), timestamp))
            .record_audit(false);
    }

    pub fn record_peer_storage(
        &mut self,
        peer_id: &str,
        contributed_delta: u64,
        consumed_delta: u64,
    ) {
        let entry = self
            .peer_credits
            .entry(peer_id.to_string())
            .or_insert_with(|| mesh_core::CreditLedger::new(peer_id.to_string(), 0));
        if contributed_delta > 0 {
            entry.record_storage_contribution(contributed_delta);
        }
        if consumed_delta > 0 {
            entry.record_storage_consumption(consumed_delta);
        }
    }

    pub fn is_peer_throttled(&self, peer_id: &str) -> bool {
        if let Some(ledger) = self.peer_credits.get(peer_id) {
            let tier = ledger.evaluate_tier(1_073_741_824, 1.0, 50_000_000_000);
            tier == mesh_core::ReciprocityTier::Throttled
                || tier == mesh_core::ReciprocityTier::Suspended
        } else {
            false
        }
    }

    pub fn get_peer_reliability(&self, peer_id: &str) -> f64 {
        self.peer_reliability
            .get(peer_id)
            .map(|t| t.reliability_score())
            .unwrap_or(1.0)
    }

    pub fn is_peer_healthy(&self, peer_id: &str) -> bool {
        self.peer_reliability
            .get(peer_id)
            .map(|t| t.is_healthy(0.5))
            .unwrap_or(true)
    }

    pub fn check_manifest_health(&self, manifest: &FileManifest) -> Vec<mesh_core::DegradedChunk> {
        let mut degraded_chunks = Vec::new();

        for (chunk_idx, chunk) in manifest.chunks.iter().enumerate() {
            let mut surviving_indices = Vec::new();

            for (shard_idx, holder) in chunk.shard_holders.iter().enumerate() {
                let is_available = if holder == &self.peer_id.to_string() {
                    let hash_hex = hex::encode(chunk.shard_hashes[shard_idx]);
                    self.has_shard(&hash_hex)
                } else if let Ok(peer_id) = holder.parse::<PeerId>() {
                    self.is_trusted(&peer_id)
                        && self.connected_peers.contains(&peer_id)
                        && self.is_peer_healthy(holder)
                } else {
                    false
                };

                if is_available {
                    surviving_indices.push(shard_idx);
                }
            }

            let degraded = mesh_core::DegradedChunk::new(
                manifest.file_id.clone(),
                chunk_idx,
                manifest.k,
                manifest.m,
                surviving_indices,
            );

            if degraded.is_degraded() {
                degraded_chunks.push(degraded);
            }
        }

        degraded_chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_node(_port: u16, quota_bytes: u64) -> (NodeState, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir created");
        let path = temp_dir.path().to_path_buf();
        let key = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(key.public());

        let mut node = NodeState::with_data_dir(peer_id, path, 1.0);
        node.set_quota(quota_bytes);
        (node, temp_dir)
    }

    #[test]
    fn test_validate_hash_key() {
        assert!(
            NodeState::validate_hash_key(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            )
            .is_ok()
        );
        assert!(NodeState::validate_hash_key("").is_err());
        assert!(NodeState::validate_hash_key("../etc/passwd").is_err());
        assert!(NodeState::validate_hash_key("not a hex string!").is_err());
    }

    #[test]
    fn test_atomic_write_and_read_shard() {
        let (mut node, _temp) = temp_test_node(9901, 10_000);
        let hash = "abcd1234ef";
        let data = b"encrypted shard atomic content";

        node.write_shard(hash, data).expect("write succeeds");
        assert!(node.has_shard(hash));
        let read = node.read_shard(hash).expect("read succeeds");
        assert_eq!(read, data);
        assert_eq!(node.storage_used, data.len() as u64);
    }

    #[test]
    fn test_quota_overflow_rejection() {
        let (mut node, _temp) = temp_test_node(9902, 100);
        let hash1 = "1111";
        let hash2 = "2222";

        node.write_shard(hash1, &[0u8; 80])
            .expect("first write succeeds");
        let err = node.write_shard(hash2, &[0u8; 30]).unwrap_err();
        assert!(err.contains("quota exceeded"));
    }

    #[test]
    fn test_pause_and_resume_lifecycle() {
        let (mut node, _temp) = temp_test_node(9903, 10_000);
        assert_eq!(node.state, NodeLifecycleState::Active);

        node.pause().unwrap();
        assert_eq!(node.state, NodeLifecycleState::Paused);

        let err = node.write_shard("3333", b"test").unwrap_err();
        assert_eq!(err, "Node is currently paused");

        node.resume().unwrap();
        assert_eq!(node.state, NodeLifecycleState::Active);
        node.write_shard("3333", b"test")
            .expect("resumed node writes shard");
    }

    #[test]
    fn test_nonce_single_use_enforcement() {
        let (mut node, _temp) = temp_test_node(9904, 10_000);
        let nonce = "unique_nonce_abc_123";

        // First consume must succeed
        assert!(node.consume_nonce(nonce).is_ok());
        assert!(node.consumed_nonces.contains(nonce));

        // Replay of same nonce must fail
        let err = node.consume_nonce(nonce).unwrap_err();
        assert_eq!(err, "Invitation nonce has already been used");
    }
}
