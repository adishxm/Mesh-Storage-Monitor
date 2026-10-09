use anyhow::Result;
use libp2p::identity::Keypair;
use std::fs;
use std::path::Path;

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
}
