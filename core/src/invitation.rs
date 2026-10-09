use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum InvitationError {
    #[error("Invitation expired at {expired_at}, current time is {current_time}")]
    Expired { expired_at: u64, current_time: u64 },
    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),
    #[error("Invalid signature format: {0}")]
    InvalidSignature(String),
    #[error("Cryptographic signature verification failed")]
    SignatureVerificationFailed,
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    #[error("Invalid invitation payload: {0}")]
    InvalidPayload(String),
}

/// Cryptographically signed invitation for enrolling a new peer into the mesh.
/// Conforms to Section 4 of `interface_contracts.md`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub invitation_id: String,
    pub network_id: String,
    pub organization_id: String,
    pub issuer_peer_id: String,
    pub issuer_pubkey_hex: String,
    pub issuer_signature_hex: String,
    pub bootstrap_addrs: Vec<String>,
    pub expires_at: u64,
    pub single_use_nonce: String,
}

impl Invitation {
    /// Creates a new unsigned invitation with a random ID and single-use nonce.
    pub fn new(
        network_id: String,
        organization_id: String,
        issuer_peer_id: String,
        issuer_pubkey: &[u8; 32],
        bootstrap_addrs: Vec<String>,
        created_at: u64,
        validity_duration_secs: u64,
    ) -> Self {
        let mut nonce_bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let single_use_nonce = hex::encode(nonce_bytes);

        let mut id_bytes = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut id_bytes);
        let invitation_id = format!("inv_{}", hex::encode(id_bytes));

        Self {
            invitation_id,
            network_id,
            organization_id,
            issuer_peer_id,
            issuer_pubkey_hex: hex::encode(issuer_pubkey),
            issuer_signature_hex: String::new(),
            bootstrap_addrs,
            expires_at: created_at + validity_duration_secs,
            single_use_nonce,
        }
    }

    /// Computes the canonical SHA-256 digest of the invitation payload for signing/verification.
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"MSH_INVITE_V1:");
        hasher.update(self.invitation_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.network_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.organization_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.issuer_peer_id.as_bytes());
        hasher.update(b"|");
        hasher.update(self.issuer_pubkey_hex.as_bytes());
        hasher.update(b"|");
        for addr in &self.bootstrap_addrs {
            hasher.update(addr.as_bytes());
            hasher.update(b",");
        }
        hasher.update(b"|");
        hasher.update(self.expires_at.to_le_bytes());
        hasher.update(b"|");
        hasher.update(self.single_use_nonce.as_bytes());
        hasher.finalize().into()
    }

    /// Cryptographically signs the invitation with the issuer's Ed25519 signing key.
    pub fn sign(&mut self, signing_key: &SigningKey) {
        let digest = self.digest();
        let signature = signing_key.sign(&digest);
        self.issuer_signature_hex = hex::encode(signature.to_bytes());
    }

    /// Verifies that the invitation is not expired and that the cryptographic signature is valid.
    pub fn verify(&self, current_time: u64) -> Result<(), InvitationError> {
        if self.invitation_id.is_empty() {
            return Err(InvitationError::InvalidPayload(
                "Missing invitation_id".into(),
            ));
        }
        if self.single_use_nonce.is_empty() {
            return Err(InvitationError::InvalidPayload(
                "Missing single_use_nonce".into(),
            ));
        }
        if current_time > self.expires_at {
            return Err(InvitationError::Expired {
                expired_at: self.expires_at,
                current_time,
            });
        }

        let pubkey_bytes = hex::decode(&self.issuer_pubkey_hex)
            .map_err(|e| InvitationError::InvalidPublicKey(e.to_string()))?;
        if pubkey_bytes.len() != 32 {
            return Err(InvitationError::InvalidPublicKey(
                "Public key must be 32 bytes".into(),
            ));
        }
        let mut pubkey_array = [0u8; 32];
        pubkey_array.copy_from_slice(&pubkey_bytes);

        let verifying_key = VerifyingKey::from_bytes(&pubkey_array)
            .map_err(|e| InvitationError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = hex::decode(&self.issuer_signature_hex)
            .map_err(|e| InvitationError::InvalidSignature(e.to_string()))?;
        if sig_bytes.len() != 64 {
            return Err(InvitationError::InvalidSignature(
                "Signature must be 64 bytes".into(),
            ));
        }
        let signature = Signature::from_slice(&sig_bytes)
            .map_err(|e| InvitationError::InvalidSignature(e.to_string()))?;

        let digest = self.digest();
        verifying_key
            .verify(&digest, &signature)
            .map_err(|_| InvitationError::SignatureVerificationFailed)?;

        Ok(())
    }

    /// Serializes the invitation into a compact JSON string suitable for QR codes.
    pub fn to_qr_string(&self) -> Result<String, InvitationError> {
        serde_json::to_string(self).map_err(|e| InvitationError::SerializationError(e.to_string()))
    }

    /// Deserializes an invitation from a QR code JSON string.
    pub fn from_qr_string(payload: &str) -> Result<Self, InvitationError> {
        serde_json::from_str(payload)
            .map_err(|e| InvitationError::DeserializationError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invitation_sign_and_verify_roundtrip() {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        let mut invite = Invitation::new(
            "mesh-alpha".to_string(),
            "org_default".to_string(),
            "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN".to_string(),
            verifying_key.as_bytes(),
            vec!["/ip4/192.168.1.50/tcp/4001/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN".to_string()],
            1000,
            3600,
        );

        assert_eq!(invite.expires_at, 4600);
        invite.sign(&signing_key);
        assert!(!invite.issuer_signature_hex.is_empty());

        // Verification succeeds before expiry
        assert!(invite.verify(2000).is_ok());
        assert!(invite.verify(4600).is_ok());

        // Verification fails after expiry
        let err = invite.verify(4601).unwrap_err();
        assert_eq!(
            err,
            InvitationError::Expired {
                expired_at: 4600,
                current_time: 4601
            }
        );
    }

    #[test]
    fn test_invitation_tampered_payload_rejected() {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        let mut invite = Invitation::new(
            "mesh-alpha".to_string(),
            "org_default".to_string(),
            "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN".to_string(),
            verifying_key.as_bytes(),
            vec!["/ip4/192.168.1.50/tcp/4001".to_string()],
            1000,
            3600,
        );
        invite.sign(&signing_key);

        // Tamper with nonce
        invite.single_use_nonce = "tampered_nonce_123".to_string();
        let err = invite.verify(2000).unwrap_err();
        assert_eq!(err, InvitationError::SignatureVerificationFailed);

        // Tamper with network_id
        invite.network_id = "mesh-rogue".to_string();
        let err2 = invite.verify(2000).unwrap_err();
        assert_eq!(err2, InvitationError::SignatureVerificationFailed);
    }

    #[test]
    fn test_invitation_qr_serialization_roundtrip() {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        let mut invite = Invitation::new(
            "mesh-alpha".to_string(),
            "org_default".to_string(),
            "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN".to_string(),
            verifying_key.as_bytes(),
            vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            1000,
            600,
        );
        invite.sign(&signing_key);

        let qr_string = invite.to_qr_string().unwrap();
        let parsed = Invitation::from_qr_string(&qr_string).unwrap();

        assert_eq!(parsed, invite);
        assert!(parsed.verify(1200).is_ok());
    }
}
