use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::merkle::{Hash256, hash_data};

#[derive(Error, Debug, PartialEq, Eq)]
pub enum RepairError {
    #[error("Reed-Solomon erasure error: {0}")]
    ReedSolomon(String),

    #[error(
        "Insufficient surviving shards to reconstruct: got {available}, but minimum required is {required}"
    )]
    InsufficientShards { available: usize, required: usize },

    #[error("Invalid erasure parameters: k={0}, m={1}")]
    InvalidParameters(usize, usize),

    #[error("Shard size mismatch among surviving shards")]
    ShardSizeMismatch,

    #[error("No placement candidates available satisfying failure domain constraints")]
    NoPlacementCandidates,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairState {
    Pending,
    Reconstructing,
    Repaired,
    Placed,
    Unrecoverable,
}

/// Identifies a chunk with missing or corrupt shards that requires repair.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DegradedChunk {
    pub file_id: String,
    pub chunk_idx: usize,
    pub k: usize,
    pub m: usize,
    pub total_shards: usize,
    pub surviving_indices: Vec<usize>,
    pub missing_indices: Vec<usize>,
}

impl DegradedChunk {
    pub fn new(
        file_id: String,
        chunk_idx: usize,
        k: usize,
        m: usize,
        surviving_indices: Vec<usize>,
    ) -> Self {
        let total_shards = k + m;
        let mut missing_indices = Vec::new();
        for idx in 0..total_shards {
            if !surviving_indices.contains(&idx) {
                missing_indices.push(idx);
            }
        }

        Self {
            file_id,
            chunk_idx,
            k,
            m,
            total_shards,
            surviving_indices,
            missing_indices,
        }
    }

    pub fn is_degraded(&self) -> bool {
        !self.missing_indices.is_empty()
    }

    pub fn can_repair(&self) -> bool {
        self.surviving_indices.len() >= self.k
    }

    pub fn state(&self) -> RepairState {
        if !self.is_degraded() {
            RepairState::Repaired
        } else if self.can_repair() {
            RepairState::Pending
        } else {
            RepairState::Unrecoverable
        }
    }
}

/// Reconstructs all shards (both data and parity shards) from any surviving subset of at least `k` shards.
/// Returns a complete vector of all `k + m` shards.
pub fn reconstruct_all_shards(
    shards: &[Option<Vec<u8>>],
    k: usize,
    m: usize,
) -> Result<Vec<Vec<u8>>, RepairError> {
    if k == 0 || m == 0 {
        return Err(RepairError::InvalidParameters(k, m));
    }
    if shards.len() != k + m {
        return Err(RepairError::InvalidParameters(k, m));
    }

    let available = shards.iter().filter(|s| s.is_some()).count();
    if available < k {
        return Err(RepairError::InsufficientShards {
            available,
            required: k,
        });
    }

    // Determine expected shard size from the first available shard
    let shard_size = shards
        .iter()
        .find_map(|s| s.as_ref().map(|v| v.len()))
        .unwrap_or(0);

    for s in shards.iter().flatten() {
        if s.len() != shard_size {
            return Err(RepairError::ShardSizeMismatch);
        }
    }

    let r = ReedSolomon::new(k, m).map_err(|e| RepairError::ReedSolomon(e.to_string()))?;

    let mut mutable_shards = shards.to_vec();
    r.reconstruct(&mut mutable_shards)
        .map_err(|e| RepairError::ReedSolomon(e.to_string()))?;

    let mut result = Vec::with_capacity(k + m);
    for (idx, shard_opt) in mutable_shards.into_iter().enumerate() {
        let shard_bytes = shard_opt.ok_or_else(|| {
            RepairError::ReedSolomon(format!("Failed to reconstruct shard at index {}", idx))
        })?;
        result.push(shard_bytes);
    }

    Ok(result)
}

/// Selects candidate peers for placing newly repaired shards, enforcing failure domain separation.
/// No peer may hold more than one shard for the same chunk.
pub fn select_placement_candidates(
    available_peers: &[String],
    existing_holders: &[String],
    needed_count: usize,
) -> Vec<String> {
    available_peers
        .iter()
        .filter(|peer| !existing_holders.contains(peer))
        .take(needed_count)
        .cloned()
        .collect()
}

/// Represents an executable repair assignment for a degraded chunk.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedShardPlan {
    pub shard_idx: usize,
    pub shard_hash: Hash256,
    pub shard_data: Vec<u8>,
    pub target_peer_id: String,
}

/// Generates the concrete repair plan for a degraded chunk given surviving shard bytes and available placement peers.
pub fn plan_chunk_repair(
    degraded: &DegradedChunk,
    surviving_shards: &[Option<Vec<u8>>],
    available_peers: &[String],
    existing_holders: &[String],
) -> Result<Vec<ReconstructedShardPlan>, RepairError> {
    if !degraded.can_repair() {
        return Err(RepairError::InsufficientShards {
            available: degraded.surviving_indices.len(),
            required: degraded.k,
        });
    }

    let all_shards = reconstruct_all_shards(surviving_shards, degraded.k, degraded.m)?;
    let candidates = select_placement_candidates(
        available_peers,
        existing_holders,
        degraded.missing_indices.len(),
    );

    if candidates.len() < degraded.missing_indices.len() {
        return Err(RepairError::NoPlacementCandidates);
    }

    let mut plans = Vec::with_capacity(degraded.missing_indices.len());
    for (i, &missing_idx) in degraded.missing_indices.iter().enumerate() {
        let shard_data = all_shards[missing_idx].clone();
        let shard_hash = hash_data(&shard_data);
        let target_peer_id = candidates[i].clone();

        plans.push(ReconstructedShardPlan {
            shard_idx: missing_idx,
            shard_hash,
            shard_data,
            target_peer_id,
        });
    }

    Ok(plans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::erasure::encode_data;

    #[test]
    fn test_degraded_chunk_state_transitions() {
        // k = 2, m = 1 (total 3 shards: 0, 1, 2)
        let pristine = DegradedChunk::new("file-1".into(), 0, 2, 1, vec![0, 1, 2]);
        assert!(!pristine.is_degraded());
        assert_eq!(pristine.state(), RepairState::Repaired);

        // 1 missing shard (shard 1 missing), 2 surviving (0 and 2) >= k(2) -> Pending
        let degraded = DegradedChunk::new("file-1".into(), 0, 2, 1, vec![0, 2]);
        assert!(degraded.is_degraded());
        assert!(degraded.can_repair());
        assert_eq!(degraded.missing_indices, vec![1]);
        assert_eq!(degraded.state(), RepairState::Pending);

        // 2 missing shards, only 1 surviving < k(2) -> Unrecoverable
        let unrecoverable = DegradedChunk::new("file-1".into(), 0, 2, 1, vec![0]);
        assert!(unrecoverable.is_degraded());
        assert!(!unrecoverable.can_repair());
        assert_eq!(unrecoverable.state(), RepairState::Unrecoverable);
    }

    #[test]
    fn test_reconstruct_all_shards_identity() {
        let payload = b"Cryptographically protected distributed filesystem block payload";
        let k = 3;
        let m = 2;
        let original_shards = encode_data(payload, k, m).expect("Encode shards");
        assert_eq!(original_shards.len(), 5);

        let original_hashes: Vec<Hash256> = original_shards.iter().map(|s| hash_data(s)).collect();

        // Simulate losing shard 1 (data) and shard 4 (parity)
        let mut degraded_input: Vec<Option<Vec<u8>>> =
            original_shards.into_iter().map(Some).collect();
        degraded_input[1] = None;
        degraded_input[4] = None;

        let reconstructed =
            reconstruct_all_shards(&degraded_input, k, m).expect("Reconstruct all shards");
        assert_eq!(reconstructed.len(), 5);

        // Verify reconstructed hashes are bit-for-bit identical to original
        for idx in 0..5 {
            let recon_hash = hash_data(&reconstructed[idx]);
            assert_eq!(
                recon_hash, original_hashes[idx],
                "Reconstructed shard {} hash must match original!",
                idx
            );
        }
    }

    #[test]
    fn test_failure_domain_placement_candidate_selection() {
        let available_peers = vec![
            "peer-node-1".to_string(),
            "peer-node-2".to_string(),
            "peer-node-3".to_string(),
            "peer-node-4".to_string(),
            "peer-node-5".to_string(),
        ];

        let existing_holders = vec!["peer-node-1".to_string(), "peer-node-3".to_string()];

        let candidates = select_placement_candidates(&available_peers, &existing_holders, 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates, vec!["peer-node-2", "peer-node-4"]);

        // Verify none of existing holders were selected
        for candidate in &candidates {
            assert!(!existing_holders.contains(candidate));
        }
    }

    #[test]
    fn test_plan_chunk_repair_workflow() {
        let payload = b"Mission critical block data for disaster recovery";
        let k = 2;
        let m = 1;
        let original_shards = encode_data(payload, k, m).expect("Encode");

        let degraded = DegradedChunk::new("file-audit-99".into(), 0, k, m, vec![0, 2]);
        assert_eq!(degraded.missing_indices, vec![1]);

        let surviving_inputs = vec![
            Some(original_shards[0].clone()),
            None,
            Some(original_shards[2].clone()),
        ];

        let available_peers = vec![
            "peer-host-a".to_string(),
            "peer-host-b".to_string(),
            "peer-host-c".to_string(),
        ];
        let existing_holders = vec!["peer-host-a".to_string(), "peer-host-c".to_string()];

        let plans = plan_chunk_repair(
            &degraded,
            &surviving_inputs,
            &available_peers,
            &existing_holders,
        )
        .expect("Plan chunk repair");

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].shard_idx, 1);
        assert_eq!(plans[0].target_peer_id, "peer-host-b");
        assert_eq!(plans[0].shard_hash, hash_data(&original_shards[1]));
    }
}
