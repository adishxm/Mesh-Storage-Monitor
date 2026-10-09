use libp2p::{Multiaddr, PeerId};
use mesh_core::FileManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use tracing::{error, info};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NodeStatus {
    pub peer_id: String,
    pub listen_addresses: Vec<String>,
    pub peers: Vec<String>,
    pub storage_used: u64,
    pub storage_quota: u64,
    pub shards: Vec<String>,
    pub trusted_peers: Vec<String>,
}

pub struct NodeState {
    pub peer_id: PeerId,
    pub listen_addresses: HashSet<Multiaddr>,
    pub connected_peers: HashSet<PeerId>,
    pub trusted_peers: HashSet<PeerId>,
    pub storage_quota: u64,
    pub storage_used: u64,
    pub data_dir: PathBuf,
}

#[allow(dead_code)]
impl NodeState {
    pub fn new(peer_id: PeerId, port: u16, quota_gb: f64) -> Self {
        let data_dir = PathBuf::from(format!("./data_{}", port));
        fs::create_dir_all(data_dir.join("shards")).unwrap_or_default();
        fs::create_dir_all(data_dir.join("manifests")).unwrap_or_default();

        let quota_bytes = (quota_gb * 1024.0 * 1024.0 * 1024.0) as u64;

        let mut state = Self {
            peer_id,
            listen_addresses: HashSet::new(),
            connected_peers: HashSet::new(),
            trusted_peers: HashSet::new(),
            storage_quota: quota_bytes,
            storage_used: 0,
            data_dir,
        };

        state.load_trusted_peers();
        state.recalculate_storage_used();
        state
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
        info!(
            "Storage used: {} / {} bytes",
            self.storage_used, self.storage_quota
        );
    }

    pub fn has_shard(&self, hash_hex: &str) -> bool {
        self.data_dir.join("shards").join(hash_hex).exists()
    }

    pub fn read_shard(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let path = self.data_dir.join("shards").join(hash_hex);
        if path.exists() {
            fs::read(path).ok()
        } else {
            None
        }
    }

    pub fn write_shard(&mut self, hash_hex: &str, data: &[u8]) -> Result<(), String> {
        if self.storage_used + data.len() as u64 > self.storage_quota {
            return Err("Storage quota exceeded".to_string());
        }

        let path = self.data_dir.join("shards").join(hash_hex);
        fs::write(path, data).map_err(|e| e.to_string())?;
        self.recalculate_storage_used();
        Ok(())
    }

    pub fn delete_shard(&mut self, hash_hex: &str) -> Result<(), String> {
        let path = self.data_dir.join("shards").join(hash_hex);
        if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
            self.recalculate_storage_used();
        }
        Ok(())
    }

    pub fn save_manifest(&self, manifest: &FileManifest) -> Result<(), String> {
        let path = self
            .data_dir
            .join("manifests")
            .join(format!("{}.json", manifest.file_id));
        let content = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn read_manifest(&self, file_id: &str) -> Option<FileManifest> {
        let path = self
            .data_dir
            .join("manifests")
            .join(format!("{}.json", file_id));
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

    pub fn get_status(&self) -> NodeStatus {
        let shards = fs::read_dir(self.data_dir.join("shards"))
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();

        NodeStatus {
            peer_id: self.peer_id.to_string(),
            listen_addresses: self
                .listen_addresses
                .iter()
                .map(|a| a.to_string())
                .collect(),
            peers: self.connected_peers.iter().map(|p| p.to_string()).collect(),
            storage_used: self.storage_used,
            storage_quota: self.storage_quota,
            shards,
            trusted_peers: self.trusted_peers.iter().map(|p| p.to_string()).collect(),
        }
    }
}
