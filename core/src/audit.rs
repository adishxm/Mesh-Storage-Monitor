use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::merkle::{Hash256, hash_data};

/// A proof-of-storage cryptographic challenge sent to a storage peer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AuditChallenge {
    pub challenge_id: String,
    pub shard_hash: Hash256,
    pub nonce: [u8; 16],
    pub created_at: u64,
}

impl AuditChallenge {
    pub fn new(shard_hash: Hash256) -> Self {
        let mut nonce = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut nonce);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let challenge_id = hex::encode(rand::random::<[u8; 8]>());

        Self {
            challenge_id,
            shard_hash,
            nonce,
            created_at: now,
        }
    }

    pub fn with_nonce(
        challenge_id: String,
        shard_hash: Hash256,
        nonce: [u8; 16],
        created_at: u64,
    ) -> Self {
        Self {
            challenge_id,
            shard_hash,
            nonce,
            created_at,
        }
    }
}

/// A response proof returned by a peer claiming possession of a shard.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct AuditProof {
    pub challenge_id: String,
    pub shard_hash: Hash256,
    pub proof_hash: Hash256,
    pub responder_peer_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum AuditVerificationResult {
    Success,
    ProofMismatch {
        expected: Hash256,
        received: Hash256,
    },
    CorruptData,
}

/// Computes the deterministic proof-of-storage hash over shard bytes and challenge nonce.
pub fn compute_audit_proof(shard_data: &[u8], nonce: &[u8; 16]) -> Hash256 {
    let mut combined = Vec::with_capacity(shard_data.len() + 16);
    combined.extend_from_slice(shard_data);
    combined.extend_from_slice(nonce);
    hash_data(&combined)
}

/// Verifies that an audit proof correctly proves possession of genuine shard data.
pub fn verify_audit_proof(
    shard_data: &[u8],
    challenge: &AuditChallenge,
    proof: &AuditProof,
) -> AuditVerificationResult {
    if challenge.shard_hash != proof.shard_hash {
        return AuditVerificationResult::ProofMismatch {
            expected: challenge.shard_hash,
            received: proof.shard_hash,
        };
    }

    let actual_shard_hash = hash_data(shard_data);
    if actual_shard_hash != challenge.shard_hash {
        return AuditVerificationResult::CorruptData;
    }

    let expected_proof = compute_audit_proof(shard_data, &challenge.nonce);
    if expected_proof == proof.proof_hash {
        AuditVerificationResult::Success
    } else {
        AuditVerificationResult::ProofMismatch {
            expected: expected_proof,
            received: proof.proof_hash,
        }
    }
}

/// Tracks the reliability and audit pass/fail score of a remote storage peer over time.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PeerReliabilityTracker {
    pub peer_id: String,
    pub audits_passed: u64,
    pub audits_failed: u64,
    pub consecutive_failures: u32,
    pub consecutive_passes: u32,
    pub last_audit_timestamp: u64,
}

impl PeerReliabilityTracker {
    pub fn new(peer_id: String) -> Self {
        Self {
            peer_id,
            audits_passed: 0,
            audits_failed: 0,
            consecutive_failures: 0,
            consecutive_passes: 0,
            last_audit_timestamp: 0,
        }
    }

    pub fn record_success(&mut self, timestamp: u64) {
        self.audits_passed += 1;
        self.consecutive_passes += 1;
        self.consecutive_failures = 0;
        self.last_audit_timestamp = timestamp;
    }

    pub fn record_failure(&mut self, timestamp: u64) {
        self.audits_failed += 1;
        self.consecutive_failures += 1;
        self.consecutive_passes = 0;
        self.last_audit_timestamp = timestamp;
    }

    /// Computes a smoothed Bayesian reliability score in [0.0, 1.0].
    /// Applies a progressive penalty for consecutive failures.
    pub fn reliability_score(&self) -> f64 {
        let total = self.audits_passed + self.audits_failed;
        if total == 0 {
            return 1.0; // New peer starts with neutral credit
        }

        // Laplace smoothing (prior alpha = 2, beta = 1)
        let base_score = (self.audits_passed as f64 + 2.0) / (total as f64 + 3.0);

        if self.consecutive_failures >= 3 {
            // Rapid degradation if failing consecutive challenges
            (base_score * 0.2).clamp(0.0, 1.0)
        } else if self.consecutive_failures > 0 {
            (base_score * (1.0 - 0.25 * self.consecutive_failures as f64)).clamp(0.0, 1.0)
        } else {
            base_score.clamp(0.0, 1.0)
        }
    }

    /// Determines if a peer meets the minimum health threshold for storing shards.
    pub fn is_healthy(&self, min_score: f64) -> bool {
        self.consecutive_failures < 3 && self.reliability_score() >= min_score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_proof_computation_and_verification_success() {
        let shard_data = b"Encrypted Reed-Solomon shard payload with confidential bytes";
        let shard_hash = hash_data(shard_data);

        let challenge = AuditChallenge::new(shard_hash);
        let proof_hash = compute_audit_proof(shard_data, &challenge.nonce);

        let proof = AuditProof {
            challenge_id: challenge.challenge_id.clone(),
            shard_hash,
            proof_hash,
            responder_peer_id: "peer-storage-1".to_string(),
        };

        let result = verify_audit_proof(shard_data, &challenge, &proof);
        assert_eq!(result, AuditVerificationResult::Success);
    }

    #[test]
    fn test_tampered_shard_detected_as_corrupt() {
        let original_shard_data = b"Original pristine shard data";
        let shard_hash = hash_data(original_shard_data);

        let challenge = AuditChallenge::new(shard_hash);

        // Attacker stored corrupt / flipped bits
        let corrupt_shard_data = b"Altered corrupted shard data";
        let attacker_proof_hash = compute_audit_proof(corrupt_shard_data, &challenge.nonce);

        let proof = AuditProof {
            challenge_id: challenge.challenge_id.clone(),
            shard_hash,
            proof_hash: attacker_proof_hash,
            responder_peer_id: "peer-untrusted".to_string(),
        };

        let result = verify_audit_proof(corrupt_shard_data, &challenge, &proof);
        assert_eq!(result, AuditVerificationResult::CorruptData);
    }

    #[test]
    fn test_invalid_nonce_detected_as_proof_mismatch() {
        let shard_data = b"Valid shard payload bytes";
        let shard_hash = hash_data(shard_data);

        let challenge = AuditChallenge::new(shard_hash);

        // Attacker computed proof with wrong nonce
        let wrong_nonce = [99u8; 16];
        let bad_proof_hash = compute_audit_proof(shard_data, &wrong_nonce);

        let proof = AuditProof {
            challenge_id: challenge.challenge_id.clone(),
            shard_hash,
            proof_hash: bad_proof_hash,
            responder_peer_id: "peer-bad-nonce".to_string(),
        };

        let result = verify_audit_proof(shard_data, &challenge, &proof);
        assert!(matches!(
            result,
            AuditVerificationResult::ProofMismatch { .. }
        ));
    }

    #[test]
    fn test_peer_reliability_tracker_and_penalty() {
        let mut tracker = PeerReliabilityTracker::new("peer-node-alpha".to_string());
        assert_eq!(tracker.reliability_score(), 1.0);
        assert!(tracker.is_healthy(0.7));

        // 10 successful audits
        for i in 1..=10 {
            tracker.record_success(1700000000 + i);
        }
        assert!(tracker.reliability_score() > 0.85);
        assert_eq!(tracker.consecutive_passes, 10);
        assert_eq!(tracker.consecutive_failures, 0);

        // 1 failure decreases score mildly
        tracker.record_failure(1700000011);
        assert_eq!(tracker.consecutive_failures, 1);
        assert!(tracker.is_healthy(0.5));

        // 3 consecutive failures drastically penalizes score and marks unhealthy
        tracker.record_failure(1700000012);
        tracker.record_failure(1700000013);
        assert_eq!(tracker.consecutive_failures, 3);
        assert!(!tracker.is_healthy(0.5));
        assert!(tracker.reliability_score() < 0.25);

        // Recovery with successful audits
        tracker.record_success(1700000014);
        tracker.record_success(1700000015);
        tracker.record_success(1700000016);
        assert_eq!(tracker.consecutive_failures, 0);
        assert!(tracker.is_healthy(0.5));
    }
}
