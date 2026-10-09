use anyhow::Result;
use ed25519_dalek::SigningKey;
use libp2p::identity::Keypair;
use mesh_core::Invitation;
use rand::RngCore;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Loads an existing Ed25519 keypair from `<data_dir>/identity.key` or generates a new one.
pub fn load_or_create_keypair(data_dir: &Path) -> Result<Keypair> {
    let path = data_dir.join("identity.key");
    if path.exists() {
        let bytes = fs::read(&path)?;
        let key = Keypair::from_protobuf_encoding(&bytes)?;
        Ok(key)
    } else {
        let key = Keypair::generate_ed25519();
        let bytes = key.to_protobuf_encoding()?;
        fs::write(&path, bytes)?;
        Ok(key)
    }
}

/// Loads or generates a persistent Ed25519 signing key used to sign mesh invitations.
pub fn get_or_create_invite_signing_key(data_dir: &Path) -> Result<SigningKey> {
    let path = data_dir.join("invite_signer.key");
    if path.exists() {
        let bytes = fs::read(&path)?;
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            return Ok(SigningKey::from_bytes(&arr));
        }
    }
    let mut secret = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    fs::write(&path, secret)?;
    Ok(SigningKey::from_bytes(&secret))
}

/// Helper that creates and signs a new Invitation for this node.
pub fn create_signed_invitation(
    data_dir: &Path,
    network_id: String,
    organization_id: String,
    issuer_peer_id: String,
    bootstrap_addrs: Vec<String>,
    validity_duration_secs: u64,
) -> Result<Invitation> {
    let signing_key = get_or_create_invite_signing_key(data_dir)?;
    let verifying_key = signing_key.verifying_key();
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut invite = Invitation::new(
        network_id,
        organization_id,
        issuer_peer_id,
        verifying_key.as_bytes(),
        bootstrap_addrs,
        current_time,
        validity_duration_secs,
    );
    invite.sign(&signing_key);
    Ok(invite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_keypair_persistence() {
        let dir = tempdir().unwrap();
        let key1 = load_or_create_keypair(dir.path()).unwrap();
        let peer_id1 = libp2p::PeerId::from(key1.public());

        // Second load from same directory must return the identical peer id
        let key2 = load_or_create_keypair(dir.path()).unwrap();
        let peer_id2 = libp2p::PeerId::from(key2.public());

        assert_eq!(peer_id1, peer_id2);
    }

    #[test]
    fn test_create_and_verify_signed_invitation() {
        let dir = tempdir().unwrap();
        let peer_id_str = "12D3KooWDpJ7As7BWAwRMfu1VU2WCqnjvq387JEYKDBj4kx6nXTN";
        let invite = create_signed_invitation(
            dir.path(),
            "mesh-alpha".to_string(),
            "org_default".to_string(),
            peer_id_str.to_string(),
            vec!["/ip4/127.0.0.1/tcp/4001".to_string()],
            3600,
        )
        .unwrap();

        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        assert!(invite.verify(current_time).is_ok());
        assert_eq!(invite.issuer_peer_id, peer_id_str);
    }
}
