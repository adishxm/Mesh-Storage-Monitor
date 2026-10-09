use mesh_core::{decode_file, encode_file, invitation::Invitation};
use mesh_node::identity::{create_signed_invitation, load_or_create_keypair};
use mesh_node::state::{NodeLifecycleState, NodeState};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

/// Helper to generate reproducible random test data
fn generate_test_payload(size: usize) -> Vec<u8> {
    let mut data = vec![0u8; size];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = ((i * 31 + 17) % 256) as u8;
    }
    data
}

#[test]
fn test_three_node_invitation_enrollment_and_replay_defense() {
    let temp_a = tempdir().expect("tempdir A");
    let temp_b = tempdir().expect("tempdir B");
    let temp_c = tempdir().expect("tempdir C");

    let key_a = load_or_create_keypair(temp_a.path()).unwrap();
    let key_b = load_or_create_keypair(temp_b.path()).unwrap();
    let key_c = load_or_create_keypair(temp_c.path()).unwrap();

    let peer_a = libp2p::PeerId::from(key_a.public());
    let peer_b = libp2p::PeerId::from(key_b.public());
    let peer_c = libp2p::PeerId::from(key_c.public());

    let mut state_a = NodeState::with_data_dir(peer_a, temp_a.path().to_path_buf(), 1.0);
    let mut state_b = NodeState::with_data_dir(peer_b, temp_b.path().to_path_buf(), 1.0);
    let mut state_c = NodeState::with_data_dir(peer_c, temp_c.path().to_path_buf(), 1.0);

    // 1. Node A (Cluster Owner) issues an invitation for Node B
    let invite_for_b = create_signed_invitation(
        temp_a.path(),
        "mesh-alpha".to_string(),
        "org_finance".to_string(),
        peer_a.to_string(),
        vec![format!("/ip4/127.0.0.1/tcp/14001/p2p/{}", peer_a)],
        3600,
    )
    .expect("Create invite for B");

    // 2. Node B joins using QR payload string
    let qr_payload = invite_for_b.to_qr_string().expect("QR serialization");
    let parsed_invite = Invitation::from_qr_string(&qr_payload).expect("QR deserialization");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Verification succeeds
    assert!(parsed_invite.verify(now).is_ok());

    // Node B consumes the nonce and trusts Node A
    assert!(
        state_b
            .consume_nonce(&parsed_invite.single_use_nonce)
            .is_ok()
    );
    state_b.add_trusted_peer(peer_a);
    state_a.add_trusted_peer(peer_b);

    assert!(state_b.is_trusted(&peer_a));
    assert!(state_a.is_trusted(&peer_b));

    // 3. Replay attack: Attacker attempts to reuse the same invitation nonce on Node B
    let replay_err = state_b
        .consume_nonce(&parsed_invite.single_use_nonce)
        .unwrap_err();
    assert_eq!(replay_err, "Invitation nonce has already been used");

    // 4. Node A issues a second invitation for Node C
    let invite_for_c = create_signed_invitation(
        temp_a.path(),
        "mesh-alpha".to_string(),
        "org_finance".to_string(),
        peer_a.to_string(),
        vec![format!("/ip4/127.0.0.1/tcp/14001/p2p/{}", peer_a)],
        3600,
    )
    .expect("Create invite for C");

    assert!(invite_for_c.verify(now).is_ok());
    assert!(
        state_c
            .consume_nonce(&invite_for_c.single_use_nonce)
            .is_ok()
    );
    state_c.add_trusted_peer(peer_a);
    state_a.add_trusted_peer(peer_c);

    // Cross-trust Node B and C
    state_b.add_trusted_peer(peer_c);
    state_c.add_trusted_peer(peer_b);

    assert!(state_a.is_trusted(&peer_b) && state_a.is_trusted(&peer_c));
    assert!(state_b.is_trusted(&peer_a) && state_b.is_trusted(&peer_c));
    assert!(state_c.is_trusted(&peer_a) && state_c.is_trusted(&peer_b));
}

#[test]
fn test_three_node_cluster_storage_distribution_and_offline_recovery() {
    let temp_a = tempdir().expect("tempdir A");
    let temp_b = tempdir().expect("tempdir B");
    let temp_c = tempdir().expect("tempdir C");

    let peer_a = libp2p::PeerId::random();
    let peer_b = libp2p::PeerId::random();
    let peer_c = libp2p::PeerId::random();

    let mut node_a = NodeState::with_data_dir(peer_a, temp_a.path().to_path_buf(), 0.5);
    let mut node_b = NodeState::with_data_dir(peer_b, temp_b.path().to_path_buf(), 0.5);
    let mut node_c = NodeState::with_data_dir(peer_c, temp_c.path().to_path_buf(), 0.5);

    // Setup 3-way mutual trust
    node_a.add_trusted_peer(peer_b);
    node_a.add_trusted_peer(peer_c);
    node_b.add_trusted_peer(peer_a);
    node_b.add_trusted_peer(peer_c);
    node_c.add_trusted_peer(peer_a);
    node_c.add_trusted_peer(peer_b);

    // 1. Prepare confidential test payload (80 KB)
    let original_payload = generate_test_payload(80 * 1024);
    let passphrase = b"MeshClusterSuperSecureMasterPassphrase!2026";
    let salt = b"ClusterSalt12345";
    let file_id = "urn:uuid:mesh-three-device-lan-mvp-file-1";
    let k = 2; // 2 data shards
    let m = 1; // 1 parity shard -> total 3 shards

    // 2. Client encodes file using mesh-core pipeline: FastCDC -> Reed-Solomon -> AES-256-GCM -> Merkle DAG
    let (manifest, encoded_chunks) =
        encode_file(&original_payload, passphrase, salt, file_id, k, m)
            .expect("Core file encoding");

    assert_eq!(manifest.chunks.len(), 1);
    let chunk_shards = &encoded_chunks[0];
    assert_eq!(
        chunk_shards.len(),
        3,
        "k=2, m=1 must produce exactly 3 shards"
    );

    let shard_0_hash = hex::encode(manifest.chunks[0].shard_hashes[0]);
    let shard_1_hash = hex::encode(manifest.chunks[0].shard_hashes[1]);
    let shard_2_hash = hex::encode(manifest.chunks[0].shard_hashes[2]);

    // 3. Failure Domain Placement: distribute 1 shard to each distinct physical node
    node_a
        .write_shard(&shard_0_hash, &chunk_shards[0])
        .expect("Store shard 0 on Node A");
    node_b
        .write_shard(&shard_1_hash, &chunk_shards[1])
        .expect("Store shard 1 on Node B");
    node_c
        .write_shard(&shard_2_hash, &chunk_shards[2])
        .expect("Store shard 2 on Node C");

    // Save manifest on Node A
    node_a
        .save_manifest(&manifest)
        .expect("Save manifest on Node A");

    // Verify all 3 nodes have their shard stored on disk and quota tracked
    assert!(node_a.has_shard(&shard_0_hash));
    assert!(node_b.has_shard(&shard_1_hash));
    assert!(node_c.has_shard(&shard_2_hash));

    assert_eq!(node_a.storage_used, chunk_shards[0].len() as u64);
    assert_eq!(node_b.storage_used, chunk_shards[1].len() as u64);
    assert_eq!(node_c.storage_used, chunk_shards[2].len() as u64);

    // -------------------------------------------------------------
    // 4. TEST REQ-13 & REQ-14: SIMULATE NODE C GOING OFFLINE / PAUSED
    // -------------------------------------------------------------
    node_c.pause().expect("Pause node C");
    assert_eq!(node_c.state, NodeLifecycleState::Paused);

    // Attempting to store on paused Node C is rejected
    let paused_reject = node_c.write_shard("deadbeef", b"data").unwrap_err();
    assert_eq!(paused_reject, "Node is currently paused");

    // Client downloads file while Node C is offline:
    // Node A provides Shard 0
    let shard_0_data = node_a.read_shard(&shard_0_hash);
    assert!(shard_0_data.is_some());

    // Node B provides Shard 1
    let shard_1_data = node_b.read_shard(&shard_1_hash);
    assert!(shard_1_data.is_some());

    // Node C is offline / unreachable (simulated as None)
    let shard_2_data: Option<Vec<u8>> = None;

    let retrieved_shards = vec![vec![shard_0_data, shard_1_data, shard_2_data]];

    // 5. Decode file with 1 offline node: Reed-Solomon reconstructs Shard 2!
    let reconstructed_payload = decode_file(&manifest, passphrase, salt, &retrieved_shards, k, m)
        .expect("Reconstruction from 2 surviving shards must succeed");

    assert_eq!(
        reconstructed_payload.len(),
        original_payload.len(),
        "Reconstructed payload length matches original"
    );
    assert_eq!(
        reconstructed_payload, original_payload,
        "Reconstructed payload bytes match original byte-for-byte"
    );

    // -------------------------------------------------------------
    // 6. TEST FAILURE RECOVERY BOUNDARY: 2 Nodes Offline (Insufficient)
    // -------------------------------------------------------------
    // If Node B ALSO fails, only 1 shard remains out of k=2
    let insufficient_shards = vec![vec![
        node_a.read_shard(&shard_0_hash),
        None, // Node B offline
        None, // Node C offline
    ]];

    let failure_err =
        decode_file(&manifest, passphrase, salt, &insufficient_shards, k, m).unwrap_err();
    assert!(
        failure_err.contains("Cannot reconstruct chunk 0: only 1 valid shards available"),
        "Must cleanly reject reconstruction when below threshold k: got {}",
        failure_err
    );

    // -------------------------------------------------------------
    // 7. TEST NODE C RECOVERY & RESUME
    // -------------------------------------------------------------
    node_c.resume().expect("Resume node C");
    assert_eq!(node_c.state, NodeLifecycleState::Active);

    // Node C shard is still intact after resumption
    assert!(node_c.has_shard(&shard_2_hash));
    assert_eq!(node_c.read_shard(&shard_2_hash).unwrap(), chunk_shards[2]);
}
