use aes_gcm::{AeadInPlace, Aes256Gcm, KeyInit, aead::generic_array::GenericArray};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::FileManifest;
use crate::credits::CreditLedger;

pub const BACKUP_MAGIC: &[u8; 4] = b"MBAK";
pub const CURRENT_BACKUP_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 4 + 2 + 16 + 12; // Magic(4) + Ver(2) + Salt(16) + Nonce(12) = 34 bytes

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BackupError {
    #[error("Invalid backup archive: length {0} too short for header")]
    ArchiveTooShort(usize),
    #[error("Invalid magic bytes in backup archive header")]
    InvalidMagic,
    #[error("Unsupported backup version: {0}")]
    UnsupportedVersion(u16),
    #[error("Key derivation failed: {0}")]
    KeyDerivationFailed(String),
    #[error("Decryption failed: incorrect passphrase or tampered archive")]
    DecryptionFailed,
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct NodeBackupSnapshot {
    pub version: u16,
    pub created_at_secs: u64,
    pub peer_id: String,
    pub storage_quota_bytes: u64,
    pub storage_used_bytes: u64,
    pub known_manifests: Vec<FileManifest>,
    pub local_credit_ledger: Option<CreditLedger>,
    pub peer_credit_ledgers: Vec<CreditLedger>,
    pub metadata: HashMap<String, String>,
}

impl NodeBackupSnapshot {
    pub fn new(
        peer_id: String,
        storage_quota_bytes: u64,
        storage_used_bytes: u64,
        created_at_secs: u64,
    ) -> Self {
        Self {
            version: CURRENT_BACKUP_VERSION,
            created_at_secs,
            peer_id,
            storage_quota_bytes,
            storage_used_bytes,
            known_manifests: Vec::new(),
            local_credit_ledger: None,
            peer_credit_ledgers: Vec::new(),
            metadata: HashMap::new(),
        }
    }
}

/// Derives a 256-bit AES key from a passphrase and salt using Argon2id.
fn derive_backup_key(passphrase: &str, salt: &[u8; 16]) -> Result<[u8; 32], BackupError> {
    let params = Params::new(19456, 2, 1, Some(32))
        .map_err(|e| BackupError::KeyDerivationFailed(e.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut derived_key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut derived_key)
        .map_err(|e| BackupError::KeyDerivationFailed(e.to_string()))?;

    Ok(derived_key)
}

/// Creates an authenticated, encrypted `.mbak` archive from a `NodeBackupSnapshot`.
pub fn create_encrypted_backup(
    snapshot: &NodeBackupSnapshot,
    passphrase: &str,
) -> Result<Vec<u8>, BackupError> {
    let plaintext_json =
        serde_json::to_vec(snapshot).map_err(|e| BackupError::SerializationError(e.to_string()))?;

    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let mut key = derive_backup_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new(GenericArray::from_slice(&key));

    let mut payload = plaintext_json;
    let tag = cipher
        .encrypt_in_place_detached(GenericArray::from_slice(&nonce), b"MBAK_AAD", &mut payload)
        .map_err(|_| BackupError::DecryptionFailed)?;

    // Zeroize sensitive key buffer from memory
    key.fill(0);

    let mut archive = Vec::with_capacity(HEADER_LEN + payload.len() + 16);
    archive.extend_from_slice(BACKUP_MAGIC);
    archive.extend_from_slice(&CURRENT_BACKUP_VERSION.to_be_bytes());
    archive.extend_from_slice(&salt);
    archive.extend_from_slice(&nonce);
    archive.extend_from_slice(&payload);
    archive.extend_from_slice(tag.as_slice());

    Ok(archive)
}

/// Decrypts and deserializes a `NodeBackupSnapshot` from an encrypted `.mbak` archive.
pub fn restore_encrypted_backup(
    archive: &[u8],
    passphrase: &str,
) -> Result<NodeBackupSnapshot, BackupError> {
    if archive.len() < HEADER_LEN + 16 {
        return Err(BackupError::ArchiveTooShort(archive.len()));
    }

    if &archive[0..4] != BACKUP_MAGIC {
        return Err(BackupError::InvalidMagic);
    }

    let version = u16::from_be_bytes([archive[4], archive[5]]);
    if version != CURRENT_BACKUP_VERSION {
        return Err(BackupError::UnsupportedVersion(version));
    }

    let salt: [u8; 16] = archive[6..22]
        .try_into()
        .map_err(|_| BackupError::ArchiveTooShort(archive.len()))?;
    let nonce: [u8; 12] = archive[22..34]
        .try_into()
        .map_err(|_| BackupError::ArchiveTooShort(archive.len()))?;

    let ciphertext_and_tag = &archive[34..];
    let tag_offset = ciphertext_and_tag.len() - 16;
    let mut ciphertext = ciphertext_and_tag[..tag_offset].to_vec();
    let tag = GenericArray::from_slice(&ciphertext_and_tag[tag_offset..]);

    let mut key = derive_backup_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new(GenericArray::from_slice(&key));

    cipher
        .decrypt_in_place_detached(
            GenericArray::from_slice(&nonce),
            b"MBAK_AAD",
            &mut ciphertext,
            tag,
        )
        .map_err(|_| {
            key.fill(0);
            BackupError::DecryptionFailed
        })?;

    // Zeroize sensitive key buffer from memory
    key.fill(0);

    let snapshot: NodeBackupSnapshot = serde_json::from_slice(&ciphertext)
        .map_err(|e| BackupError::SerializationError(e.to_string()))?;

    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_roundtrip_encryption_and_restoration() {
        let mut snapshot = NodeBackupSnapshot::new(
            "12D3KooWTestPeer".to_string(),
            10_000_000_000,
            2_500_000_000,
            1_700_000_000,
        );
        snapshot
            .metadata
            .insert("cluster_role".to_string(), "storage_worker".to_string());
        snapshot.local_credit_ledger = Some(CreditLedger::new(
            "12D3KooWTestPeer".to_string(),
            1_700_000_000,
        ));

        let passphrase = "correct_horse_battery_staple_master_key";
        let archive = create_encrypted_backup(&snapshot, passphrase).expect("create backup");

        assert_eq!(&archive[0..4], BACKUP_MAGIC);
        assert!(archive.len() > HEADER_LEN + 16);

        // Successful restore
        let restored = restore_encrypted_backup(&archive, passphrase).expect("restore backup");
        assert_eq!(restored.peer_id, "12D3KooWTestPeer");
        assert_eq!(restored.storage_quota_bytes, 10_000_000_000);
        assert_eq!(restored.storage_used_bytes, 2_500_000_000);
        assert_eq!(
            restored.metadata.get("cluster_role").map(|s| s.as_str()),
            Some("storage_worker")
        );
        assert!(restored.local_credit_ledger.is_some());
    }

    #[test]
    fn test_backup_wrong_passphrase_fails() {
        let snapshot = NodeBackupSnapshot::new("12D3KooWTestPeer".to_string(), 1000, 100, 1000);
        let archive = create_encrypted_backup(&snapshot, "secret123").expect("create backup");

        let err = restore_encrypted_backup(&archive, "wrong_password").unwrap_err();
        assert_eq!(err, BackupError::DecryptionFailed);
    }

    #[test]
    fn test_backup_tampered_ciphertext_fails() {
        let snapshot = NodeBackupSnapshot::new("12D3KooWTestPeer".to_string(), 1000, 100, 1000);
        let mut archive = create_encrypted_backup(&snapshot, "secret123").expect("create backup");

        // Tamper with a byte in ciphertext
        let last_idx = archive.len() - 1;
        archive[last_idx] ^= 0xFF;

        let err = restore_encrypted_backup(&archive, "secret123").unwrap_err();
        assert_eq!(err, BackupError::DecryptionFailed);
    }

    #[test]
    fn test_backup_invalid_magic_fails() {
        let snapshot = NodeBackupSnapshot::new("12D3KooWTestPeer".to_string(), 1000, 100, 1000);
        let mut archive = create_encrypted_backup(&snapshot, "secret123").expect("create backup");

        archive[0] = b'X';
        let err = restore_encrypted_backup(&archive, "secret123").unwrap_err();
        assert_eq!(err, BackupError::InvalidMagic);
    }
}
