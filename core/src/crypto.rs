use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Argon2 error: {0}")]
    Argon2(#[from] argon2::Error),
    #[error("AES-GCM encryption/decryption error")]
    AesGcm,
    #[error("Invalid payload size: expected at least {0} bytes, got {1}")]
    InvalidPayloadSize(usize, usize),
}

/// Derives a 32-byte master key from a passphrase and a salt using Argon2id.
pub fn derive_master_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32], CryptoError> {
    let mut master_key = [0u8; 32];
    // Argon2id parameters: 16MB memory (16384 KB), 1 iteration, 1 degree of parallelism.
    let params = Params::new(16384, 1, 1, Some(32))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2.hash_password_into(passphrase, salt, &mut master_key)?;
    Ok(master_key)
}

/// Derives a 32-byte file-specific key from a master key using HKDF-SHA256.
pub fn derive_file_key(master_key: &[u8], salt: &[u8], file_id: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(salt), master_key);
    let mut okm = [0u8; 32];
    hk.expand(file_id, &mut okm)
        .expect("32 bytes is a valid output key length for HKDF-SHA256");
    okm
}

/// Derives a deterministic 12-byte IV for a specific shard of a chunk.
pub fn derive_shard_iv(file_key: &[u8], chunk_idx: usize, shard_idx: usize) -> [u8; 12] {
    let info = format!("iv-{}-{}", chunk_idx, shard_idx);
    let hk = Hkdf::<Sha256>::new(None, file_key);
    let mut iv = [0u8; 12];
    hk.expand(info.as_bytes(), &mut iv)
        .expect("12 bytes is a valid output length for HKDF-SHA256");
    iv
}

/// Encrypts data using AES-256-GCM.
/// Returns a payload with structure: `[12-byte IV] + [16-byte Auth Tag] + [ciphertext]`.
pub fn encrypt_data(data: &[u8], key: &[u8], iv: &[u8; 12]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::AesGcm)?;
    let nonce = Nonce::from_slice(iv);

    // Encrypt the data. Aes256Gcm appends the 16-byte tag to the ciphertext.
    let ciphertext_with_tag = cipher
        .encrypt(nonce, data)
        .map_err(|_| CryptoError::AesGcm)?;

    if ciphertext_with_tag.len() < 16 {
        return Err(CryptoError::AesGcm);
    }

    // Re-structure to matching layout: IV (12) + Tag (16) + Ciphertext
    let tag_start = ciphertext_with_tag.len() - 16;
    let ciphertext = &ciphertext_with_tag[..tag_start];
    let tag = &ciphertext_with_tag[tag_start..];

    let mut payload = Vec::with_capacity(12 + 16 + ciphertext.len());
    payload.extend_from_slice(iv);
    payload.extend_from_slice(tag);
    payload.extend_from_slice(ciphertext);

    Ok(payload)
}

/// Decrypts data using AES-256-GCM from a payload structured as:
/// `[12-byte IV] + [16-byte Auth Tag] + [ciphertext]`.
pub fn decrypt_data(payload: &[u8], key: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if payload.len() < 28 {
        return Err(CryptoError::InvalidPayloadSize(28, payload.len()));
    }

    let iv = &payload[0..12];
    let tag = &payload[12..28];
    let ciphertext = &payload[28..];

    // Reconstruct the ciphertext + tag payload expected by the aes-gcm crate
    let mut ciphertext_with_tag = Vec::with_capacity(ciphertext.len() + 16);
    ciphertext_with_tag.extend_from_slice(ciphertext);
    ciphertext_with_tag.extend_from_slice(tag);

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::AesGcm)?;
    let nonce = Nonce::from_slice(iv);

    let plaintext = cipher
        .decrypt(nonce, ciphertext_with_tag.as_ref())
        .map_err(|_| CryptoError::AesGcm)?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_master_key_derivation() {
        let passphrase = b"my_super_secret_passphrase";
        let salt = b"saltysalt";
        let key1 = derive_master_key(passphrase, salt).unwrap();
        let key2 = derive_master_key(passphrase, salt).unwrap();
        assert_eq!(key1, key2);

        let key3 = derive_master_key(b"different_passphrase", salt).unwrap();
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_file_key_derivation() {
        let master_key = [7u8; 32];
        let salt = b"saltysalt";
        let file_id = b"file-12345";
        let file_key1 = derive_file_key(&master_key, salt, file_id);
        let file_key2 = derive_file_key(&master_key, salt, file_id);
        assert_eq!(file_key1, file_key2);

        let file_key3 = derive_file_key(&master_key, salt, b"file-67890");
        assert_ne!(file_key1, file_key3);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = b"Hello, decentralized mesh network storage!";
        let key = [9u8; 32];
        let iv = [0u8; 12];

        let encrypted = encrypt_data(plaintext, &key, &iv).unwrap();
        assert_eq!(encrypted.len(), 12 + 16 + plaintext.len());

        let decrypted = decrypt_data(&encrypted, &key).unwrap();
        assert_eq!(plaintext.as_slice(), decrypted.as_slice());
    }

    #[test]
    fn test_decrypt_tampered_fails() {
        let plaintext = b"Sensitive information";
        let key = [42u8; 32];
        let iv = [1u8; 12];

        let mut encrypted = encrypt_data(plaintext, &key, &iv).unwrap();
        // Tamper with the ciphertext (last byte)
        let len = encrypted.len();
        encrypted[len - 1] ^= 1;

        let result = decrypt_data(&encrypted, &key);
        assert!(result.is_err());
    }
}
