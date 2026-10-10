pub mod audit;
pub mod chunking;
pub mod credits;
pub mod crypto;
pub mod erasure;
pub mod format;
pub mod invitation;
pub mod merkle;
pub mod quota;
pub mod repair;

use crate::chunking::chunk_data;
use crate::crypto::{
    decrypt_data, derive_file_key, derive_master_key, derive_shard_iv, encrypt_data,
};
use crate::erasure::{encode_data, reconstruct_data};
use crate::merkle::{Hash256, compute_chunk_hash, compute_root_hash, hash_data, verify_shard};
pub use audit::{
    AuditChallenge, AuditProof, AuditVerificationResult, PeerReliabilityTracker,
    compute_audit_proof, verify_audit_proof,
};
pub use credits::{
    CapacityVerificationGuard, CreditLedger, ReciprocityTier, SubnetDensityGuard, SybilError,
};
pub use invitation::{Invitation, InvitationError};
pub use quota::{
    AndroidDeviceState, AndroidNetworkPolicy, AndroidPolicyDecision, AndroidPowerPolicy,
    BandwidthLimiter, QuotaError, QuotaPolicy, QuotaTracker, evaluate_android_policy,
};
pub use repair::{
    DegradedChunk, ReconstructedShardPlan, RepairError, RepairState, plan_chunk_repair,
    plan_encrypted_chunk_repair, reconstruct_all_shards, select_placement_candidates,
};
use serde::{Deserialize, Serialize};

fn default_schema_version() -> u16 {
    2
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FileManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub file_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    pub original_len: usize,
    pub root_hash: Hash256,
    #[serde(default)]
    pub k: usize,
    #[serde(default)]
    pub m: usize,
    #[serde(default)]
    pub salt_hex: String,
    #[serde(default)]
    pub created_at: u64,
    pub chunks: Vec<ChunkManifest>,
}

impl FileManifest {
    /// Validates the self-consistency of the manifest.
    /// Checks that chunk hashes match the hashes of their child shards,
    /// and that the root hash matches the hash of all chunks.
    pub fn verify_structure(&self) -> Result<(), String> {
        let mut computed_chunk_hashes = Vec::with_capacity(self.chunks.len());
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let computed_chunk_hash = compute_chunk_hash(&chunk.shard_hashes);
            if computed_chunk_hash != chunk.chunk_hash {
                return Err(format!(
                    "Chunk {} hash mismatch: expected {:?}, computed {:?}",
                    idx, chunk.chunk_hash, computed_chunk_hash
                ));
            }
            computed_chunk_hashes.push(computed_chunk_hash);
        }

        let computed_root = compute_root_hash(&computed_chunk_hashes);
        if computed_root != self.root_hash {
            return Err(format!(
                "Root hash mismatch: expected {:?}, computed {:?}",
                self.root_hash, computed_root
            ));
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChunkManifest {
    pub chunk_hash: Hash256,
    pub shard_hashes: Vec<Hash256>,
    pub shard_holders: Vec<String>,
    pub original_len: usize,
}

/// Fully encodes a file: chunking -> Reed-Solomon -> Encryption -> Merkle DAG.
/// Returns the file manifest and the encrypted shards (grouped by chunk).
#[allow(clippy::type_complexity)]
pub fn encode_file(
    data: &[u8],
    passphrase: &[u8],
    salt: &[u8],
    file_id: &str,
    k: usize,
    m: usize,
) -> Result<(FileManifest, Vec<Vec<Vec<u8>>>), String> {
    let master_key = derive_master_key(passphrase, salt).map_err(|e| e.to_string())?;
    let file_key = derive_file_key(&master_key, salt, file_id.as_bytes());

    let chunks = chunk_data(data);
    let mut chunk_manifests = Vec::new();
    let mut all_encrypted_shards = Vec::new();
    let mut chunk_hashes = Vec::new();

    for (chunk_idx, chunk) in chunks.into_iter().enumerate() {
        let original_chunk_len = chunk.len();
        // Reed-Solomon encode plaintext chunk
        let plain_shards = encode_data(&chunk, k, m).map_err(|e| e.to_string())?;

        // Encrypt each shard and hash it
        let mut encrypted_shards = Vec::new();
        let mut shard_hashes = Vec::new();
        for (shard_idx, shard) in plain_shards.into_iter().enumerate() {
            let iv = derive_shard_iv(&file_key, chunk_idx, shard_idx);
            let enc = encrypt_data(&shard, &file_key, &iv).map_err(|e| e.to_string())?;
            let hash = hash_data(&enc);
            encrypted_shards.push(enc);
            shard_hashes.push(hash);
        }

        let chunk_hash = compute_chunk_hash(&shard_hashes);
        chunk_hashes.push(chunk_hash);
        chunk_manifests.push(ChunkManifest {
            chunk_hash,
            shard_hashes,
            shard_holders: vec![String::new(); k + m],
            original_len: original_chunk_len,
        });
        all_encrypted_shards.push(encrypted_shards);
    }

    let root_hash = compute_root_hash(&chunk_hashes);
    let salt_hex = hex::encode(salt);
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let manifest = FileManifest {
        schema_version: 2,
        file_id: file_id.to_string(),
        file_name: None,
        original_len: data.len(),
        root_hash,
        k,
        m,
        salt_hex,
        created_at,
        chunks: chunk_manifests,
    };

    Ok((manifest, all_encrypted_shards))
}

/// Fully decodes a file given its manifest, key details, and shards.
/// Shards can be missing (represented as None in the inner vector).
/// Validates the integrity of each downloaded shard against the manifest hashes
/// BEFORE decrypting and decoding.
pub fn decode_file(
    manifest: &FileManifest,
    passphrase: &[u8],
    salt: &[u8],
    shards: &[Vec<Option<Vec<u8>>>],
    k: usize,
    m: usize,
) -> Result<Vec<u8>, String> {
    manifest.verify_structure()?;

    let master_key = derive_master_key(passphrase, salt).map_err(|e| e.to_string())?;
    let file_key = derive_file_key(&master_key, salt, manifest.file_id.as_bytes());

    let mut reassembled_file = Vec::new();

    if shards.len() != manifest.chunks.len() {
        return Err(format!(
            "Chunk count mismatch: manifest has {}, provided {}",
            manifest.chunks.len(),
            shards.len()
        ));
    }

    for (chunk_idx, chunk_manifest) in manifest.chunks.iter().enumerate() {
        let provided_shards = &shards[chunk_idx];
        if provided_shards.len() != k + m {
            return Err(format!(
                "Shard count mismatch for chunk {}: expected {}, got {}",
                chunk_idx,
                k + m,
                provided_shards.len()
            ));
        }

        // 1. Verify shard integrity using the Merkle DAG hashes.
        // If a shard fails verification, treat it as missing (None) to trigger reconstruction.
        let mut verified_encrypted_shards = Vec::new();
        for (shard_idx, opt_shard) in provided_shards.iter().enumerate() {
            let expected_hash = &chunk_manifest.shard_hashes[shard_idx];
            match opt_shard {
                Some(shard_bytes) => {
                    if verify_shard(shard_bytes, expected_hash) {
                        verified_encrypted_shards.push(Some(shard_bytes.clone()));
                    } else {
                        // Integrity check failed: discard shard!
                        verified_encrypted_shards.push(None);
                    }
                }
                None => {
                    verified_encrypted_shards.push(None);
                }
            }
        }

        // 2. Count valid shards
        let valid_count = verified_encrypted_shards
            .iter()
            .filter(|s| s.is_some())
            .count();
        if valid_count < k {
            return Err(format!(
                "Cannot reconstruct chunk {}: only {} valid shards available (need at least {})",
                chunk_idx, valid_count, k
            ));
        }

        // 3. Decrypt the valid shards
        // (For the ones that are missing, we pass None to the reconstruction)
        let mut plain_shards = Vec::new();
        for opt_enc_shard in verified_encrypted_shards {
            match opt_enc_shard {
                Some(enc_bytes) => {
                    let dec = decrypt_data(&enc_bytes, &file_key).map_err(|e| e.to_string())?;
                    plain_shards.push(Some(dec));
                }
                None => {
                    plain_shards.push(None);
                }
            }
        }

        // 4. Reed-Solomon reconstruct the chunk
        let reconstructed_chunk =
            reconstruct_data(&plain_shards, k, m, chunk_manifest.original_len)
                .map_err(|e| e.to_string())?;

        reassembled_file.extend_from_slice(&reconstructed_chunk);
    }

    Ok(reassembled_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_milestone_1_done_when() {
        // 1. Generate a mock 20MB file
        let size_20mb = 20 * 1024 * 1024;
        let mut mock_file = vec![0u8; size_20mb];
        // Populate with repeating pattern to avoid trivial zeros
        for (i, byte) in mock_file.iter_mut().enumerate() {
            *byte = (i % 251) as u8;
        }

        let passphrase = b"strongpassphrase";
        let salt = b"saltsaltsalt";
        let file_id = "test-20mb-file";
        let k = 2;
        let m = 1;

        // 2. Encode the file
        let (manifest, encoded_chunks) =
            encode_file(&mock_file, passphrase, salt, file_id, k, m).unwrap();

        // 3. Normal decode (all shards present) - must be byte-identical
        let shards_all_present: Vec<Vec<Option<Vec<u8>>>> = encoded_chunks
            .iter()
            .map(|chunk_shards| chunk_shards.iter().map(|s| Some(s.clone())).collect())
            .collect();

        let decoded_normal =
            decode_file(&manifest, passphrase, salt, &shards_all_present, k, m).unwrap();
        assert_eq!(mock_file, decoded_normal);

        // 4. Missing shards test (delete any `m` shards)
        // We delete the 1st shard (index 0) of every chunk
        let mut shards_missing_some: Vec<Vec<Option<Vec<u8>>>> = shards_all_present.clone();
        for chunk_shards in &mut shards_missing_some {
            chunk_shards[0] = None;
        }

        let decoded_missing =
            decode_file(&manifest, passphrase, salt, &shards_missing_some, k, m).unwrap();
        assert_eq!(mock_file, decoded_missing);

        // 5. Corrupt shard test (a flipped bit in one shard is caught by Merkle verification before decode)
        let mut shards_corrupted = shards_all_present.clone();
        // Flipped bit in chunk 0, shard 1
        let shard_to_corrupt = &mut shards_corrupted[0][1].as_mut().unwrap();
        shard_to_corrupt[0] ^= 1;

        // Verify that decode still succeeds (because Merkle verification catches the corrupt shard,
        // discards it, and uses the remaining k=2 healthy shards to reconstruct).
        let decoded_corrupt =
            decode_file(&manifest, passphrase, salt, &shards_corrupted, k, m).unwrap();
        assert_eq!(mock_file, decoded_corrupt);

        // 6. Check that if we corrupt more than m shards, it fails
        // Flipped bit in chunk 0, shard 0 AND shard 1 (leaves only 1 healthy shard)
        let mut shards_failed = shards_all_present.clone();
        shards_failed[0][0].as_mut().unwrap()[0] ^= 1;
        shards_failed[0][1].as_mut().unwrap()[0] ^= 1;

        let decode_result = decode_file(&manifest, passphrase, salt, &shards_failed, k, m);
        assert!(decode_result.is_err());
        assert!(
            decode_result
                .unwrap_err()
                .contains("Cannot reconstruct chunk 0")
        );
    }

    #[test]
    fn test_manifest_v2_roundtrip_and_verify() {
        let data = b"Sample data for manifest v2 test";
        let salt = b"salt_12345678";
        let (manifest, _) = encode_file(data, b"pass", salt, "f-v2", 2, 1).unwrap();

        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.k, 2);
        assert_eq!(manifest.m, 1);
        assert_eq!(manifest.salt_hex, hex::encode(salt));
        assert!(manifest.verify_structure().is_ok());

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: FileManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, parsed);
        assert!(parsed.verify_structure().is_ok());
    }

    #[test]
    fn test_manifest_tampered_root_fails_verification() {
        let data = b"Integrity check test";
        let salt = b"salt_12345678";
        let (mut manifest, _) = encode_file(data, b"pass", salt, "f-tamper", 2, 1).unwrap();

        // Tamper with root hash
        manifest.root_hash[0] ^= 0xFF;
        let err = manifest.verify_structure().unwrap_err();
        assert!(err.contains("Root hash mismatch"));
    }

    #[test]
    fn test_manifest_v1_backwards_compatibility() {
        // v1 JSON lacked schema_version, k, m, salt_hex, created_at, file_name
        let v1_json = r#"{
            "file_id": "legacy_v1_file",
            "original_len": 100,
            "root_hash": [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "chunks": []
        }"#;

        let parsed: FileManifest = serde_json::from_str(v1_json).expect("v1 parses cleanly");
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(parsed.file_id, "legacy_v1_file");
        assert_eq!(parsed.original_len, 100);
        assert!(parsed.file_name.is_none());
        assert_eq!(parsed.k, 0);
        assert_eq!(parsed.m, 0);
    }
}
