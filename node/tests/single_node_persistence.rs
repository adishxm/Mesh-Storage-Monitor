use mesh_core::{ChunkManifest, FileManifest, merkle::hash_data};
use mesh_node::identity::load_or_create_keypair;
use mesh_node::state::{NodeLifecycleState, NodeState};
use tempfile::tempdir;

#[test]
fn test_single_node_identity_persistence() {
    let temp_dir = tempdir().expect("Failed to create tempdir");
    let data_path = temp_dir.path();

    // 1. First run generates new identity
    let keypair_1 = load_or_create_keypair(data_path).expect("Failed to create keypair");
    let peer_id_1 = libp2p::PeerId::from(keypair_1.public());

    // 2. Restart simulation reads existing identity file
    let keypair_2 = load_or_create_keypair(data_path).expect("Failed to reload keypair");
    let peer_id_2 = libp2p::PeerId::from(keypair_2.public());

    assert_eq!(
        peer_id_1, peer_id_2,
        "Peer ID must persist across node restarts"
    );
}

#[test]
fn test_single_node_restart_and_state_persistence() {
    let temp_dir = tempdir().expect("Failed to create tempdir");
    let data_path = temp_dir.path().to_path_buf();

    let keypair = load_or_create_keypair(&data_path).expect("Keypair setup");
    let peer_id = libp2p::PeerId::from(keypair.public());

    let shard_a_hash_str = "1111111111111111111111111111111111111111111111111111111111111111";
    let shard_a_data = b"Encrypted shard A payload for persistence test";

    let shard_b_hash_str = "2222222222222222222222222222222222222222222222222222222222222222";
    let shard_b_data = b"Encrypted shard B payload with different contents";

    let shard_a_hash = hash_data(shard_a_data);
    let shard_b_hash = hash_data(shard_b_data);

    let test_manifest = FileManifest {
        schema_version: 2,
        file_id: "test-file-persisted-123".to_string(),
        file_name: Some("document.pdf".to_string()),
        original_len: 4096,
        root_hash: [3u8; 32],
        k: 1,
        m: 1,
        salt_hex: "0102030405060708".to_string(),
        created_at: 1728500000,
        chunks: vec![ChunkManifest {
            chunk_hash: [4u8; 32],
            shard_hashes: vec![shard_a_hash, shard_b_hash],
            shard_holders: vec![],
            original_len: 4096,
        }],
    };

    let foreign_peer = libp2p::PeerId::random();

    // --- SCOPE 1: Node 1 runs, writes state, then terminates ---
    let total_written_bytes = {
        let mut node = NodeState::with_data_dir(peer_id, data_path.clone(), 1.0);
        assert_eq!(node.state, NodeLifecycleState::Active);

        // Store shards
        node.write_shard(shard_a_hash_str, shard_a_data)
            .expect("Write shard A");
        node.write_shard(shard_b_hash_str, shard_b_data)
            .expect("Write shard B");

        let expected_bytes = (shard_a_data.len() + shard_b_data.len()) as u64;
        assert_eq!(node.storage_used, expected_bytes);
        assert_eq!(node.quota_tracker.used_bytes, expected_bytes);

        // Store manifest
        node.save_manifest(&test_manifest).expect("Save manifest");

        // Add trusted peer
        node.add_trusted_peer(foreign_peer);

        expected_bytes
    };

    // --- SCOPE 2: Daemon restarts on the exact same data_path ---
    {
        let mut restarted_node = NodeState::with_data_dir(peer_id, data_path.clone(), 1.0);

        // 1. Shards and storage quota tracking must be reconstructed accurately
        assert_eq!(
            restarted_node.storage_used, total_written_bytes,
            "Recovered storage used must match pre-restart byte count"
        );
        assert_eq!(
            restarted_node.quota_tracker.used_bytes, total_written_bytes,
            "Recovered quota tracker must match pre-restart byte count"
        );
        assert!(restarted_node.has_shard(shard_a_hash_str));
        assert!(restarted_node.has_shard(shard_b_hash_str));
        assert_eq!(
            restarted_node.read_shard(shard_a_hash_str).as_deref(),
            Some(&shard_a_data[..])
        );
        assert_eq!(
            restarted_node.read_shard(shard_b_hash_str).as_deref(),
            Some(&shard_b_data[..])
        );

        // 2. Manifest persistence
        let manifests = restarted_node.list_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].file_id, "test-file-persisted-123");
        assert_eq!(manifests[0].schema_version, 2);

        let retrieved = restarted_node.read_manifest("test-file-persisted-123");
        assert!(retrieved.is_some());
        assert_eq!(
            retrieved.unwrap().file_name,
            Some("document.pdf".to_string())
        );

        // 3. Trusted peers persistence
        assert!(restarted_node.is_trusted(&foreign_peer));
        assert!(restarted_node.is_trusted(&peer_id));

        // 4. Shard deletion and dynamic quota tracking post-restart
        restarted_node
            .delete_shard(shard_a_hash_str)
            .expect("Delete shard A");
        assert!(!restarted_node.has_shard(shard_a_hash_str));
        assert_eq!(
            restarted_node.storage_used,
            shard_b_data.len() as u64,
            "Storage used must decrement after deletion"
        );
    }
}
