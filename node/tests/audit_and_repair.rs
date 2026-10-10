use mesh_core::{
    AuditChallenge, AuditProof, AuditVerificationResult, DegradedChunk, RepairError, RepairState,
    compute_audit_proof,
    crypto::{derive_file_key, derive_master_key},
    decode_file, encode_file,
    merkle::hash_data,
    plan_chunk_repair, plan_encrypted_chunk_repair, verify_audit_proof,
};
use mesh_node::state::NodeState;
use tempfile::tempdir;

#[test]
fn test_proof_of_storage_audit_flow() {
    let tmp = tempdir().expect("tempdir");
    let key = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = libp2p::PeerId::from(key.public());
    let mut state = NodeState::with_data_dir(peer_id, tmp.path().to_path_buf(), 1.0);

    let shard_payload = b"Top secret encrypted shard data stored on local node for audit testing";
    let shard_hash = hash_data(shard_payload);
    let hash_hex = hex::encode(shard_hash);

    state
        .write_shard(&hash_hex, shard_payload)
        .expect("Write shard");

    // 1. Challenger issues audit challenge
    let challenge = AuditChallenge::new(shard_hash);

    // 2. Storage node proves possession by reading shard and computing proof
    let retrieved_shard = state.read_shard(&hash_hex).expect("Read shard");
    let proof_hash = compute_audit_proof(&retrieved_shard, &challenge.nonce);

    let proof = AuditProof {
        challenge_id: challenge.challenge_id.clone(),
        shard_hash,
        proof_hash,
        responder_peer_id: peer_id.to_string(),
    };

    // 3. Verification succeeds
    let verify_res = verify_audit_proof(&retrieved_shard, &challenge, &proof);
    assert_eq!(verify_res, AuditVerificationResult::Success);

    // 4. Record success on peer reliability tracker
    state.record_audit_success(&peer_id.to_string(), challenge.created_at);
    assert_eq!(state.get_peer_reliability(&peer_id.to_string()), 0.75);
    assert!(state.is_peer_healthy(&peer_id.to_string()));
}

#[test]
fn test_tampered_shard_fails_audit_and_penalizes_peer() {
    let tmp = tempdir().expect("tempdir");
    let key = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = libp2p::PeerId::from(key.public());
    let mut state = NodeState::with_data_dir(peer_id, tmp.path().to_path_buf(), 1.0);

    let untrusted_peer = "12D3KooWBadActorPeer1234567890abcdef";

    let original_data = b"Original pristine shard data before silent corruption";
    let shard_hash = hash_data(original_data);

    let challenge = AuditChallenge::new(shard_hash);

    // Storage peer returns proof of corrupted data
    let corrupt_data = b"Bit-rotted corrupted shard data payload";
    let bad_proof_hash = compute_audit_proof(corrupt_data, &challenge.nonce);

    let proof = AuditProof {
        challenge_id: challenge.challenge_id.clone(),
        shard_hash,
        proof_hash: bad_proof_hash,
        responder_peer_id: untrusted_peer.to_string(),
    };

    let verify_res = verify_audit_proof(corrupt_data, &challenge, &proof);
    assert_eq!(verify_res, AuditVerificationResult::CorruptData);

    // Penalize peer with 3 consecutive audit failures
    for i in 1..=3 {
        state.record_audit_failure(untrusted_peer, challenge.created_at + i);
    }

    assert!(!state.is_peer_healthy(untrusted_peer));
    assert!(state.get_peer_reliability(untrusted_peer) < 0.3);
}

#[test]
fn test_automated_shard_repair_and_failure_domain_placement() {
    let tmp_a = tempdir().expect("tempdir A");
    let tmp_b = tempdir().expect("tempdir B");
    let tmp_c = tempdir().expect("tempdir C");
    let tmp_d = tempdir().expect("tempdir D (spare placement node)");

    let key_a = libp2p::identity::Keypair::generate_ed25519();
    let key_b = libp2p::identity::Keypair::generate_ed25519();
    let key_c = libp2p::identity::Keypair::generate_ed25519();
    let key_d = libp2p::identity::Keypair::generate_ed25519();

    let peer_a = libp2p::PeerId::from(key_a.public());
    let peer_b = libp2p::PeerId::from(key_b.public());
    let peer_c = libp2p::PeerId::from(key_c.public());
    let peer_d = libp2p::PeerId::from(key_d.public());

    let mut state_a = NodeState::with_data_dir(peer_a, tmp_a.path().to_path_buf(), 1.0);
    let mut state_b = NodeState::with_data_dir(peer_b, tmp_b.path().to_path_buf(), 1.0);
    let mut state_c = NodeState::with_data_dir(peer_c, tmp_c.path().to_path_buf(), 1.0);
    let mut state_d = NodeState::with_data_dir(peer_d, tmp_d.path().to_path_buf(), 1.0);

    // Cross-trust peers
    state_a.add_trusted_peer(peer_b);
    state_a.add_trusted_peer(peer_c);
    state_a.add_trusted_peer(peer_d);
    state_a.connected_peers.insert(peer_b);
    state_a.connected_peers.insert(peer_c);
    state_a.connected_peers.insert(peer_d);

    // 1. Encode file with k=2, m=1 (3 shards)
    let secret_file = b"Distributed enterprise confidential records that must survive node churn";
    let passphrase = b"repair-secure-passphrase";
    let salt = [42u8; 16];
    let k = 2;
    let m = 1;

    let (manifest, shards_by_chunk) =
        encode_file(secret_file, passphrase, &salt, "finance.db", k, m).expect("Encode file");

    assert_eq!(shards_by_chunk.len(), 1);
    let chunk_shards = &shards_by_chunk[0];
    assert_eq!(chunk_shards.len(), 3);

    // Store Shard 0 on Node A, Shard 1 on Node B, Shard 2 on Node C
    let hash_0 = hex::encode(manifest.chunks[0].shard_hashes[0]);
    let hash_1 = hex::encode(manifest.chunks[0].shard_hashes[1]);
    let hash_2 = hex::encode(manifest.chunks[0].shard_hashes[2]);

    state_a.write_shard(&hash_0, &chunk_shards[0]).unwrap();
    state_b.write_shard(&hash_1, &chunk_shards[1]).unwrap();
    state_c.write_shard(&hash_2, &chunk_shards[2]).unwrap();

    let mut tracked_manifest = manifest.clone();
    tracked_manifest.chunks[0].shard_holders =
        vec![peer_a.to_string(), peer_b.to_string(), peer_c.to_string()];

    // 2. Simulate Node B failing (consecutive audit failures -> marked unhealthy)
    state_a.record_audit_failure(&peer_b.to_string(), 100);
    state_a.record_audit_failure(&peer_b.to_string(), 101);
    state_a.record_audit_failure(&peer_b.to_string(), 102);
    assert!(!state_a.is_peer_healthy(&peer_b.to_string()));

    // 3. Automated health inspection detects degraded chunk
    let degraded = state_a.check_manifest_health(&tracked_manifest);
    assert_eq!(degraded.len(), 1);
    assert_eq!(degraded[0].chunk_idx, 0);
    assert_eq!(degraded[0].missing_indices, vec![1]);
    assert_eq!(degraded[0].surviving_indices, vec![0, 2]);
    assert!(degraded[0].can_repair());

    // 4. Prepare surviving shard inputs for Reed-Solomon repair
    let surviving_inputs = vec![
        Some(state_a.read_shard(&hash_0).unwrap()),
        None, // Shard 1 lost
        Some(state_c.read_shard(&hash_2).unwrap()),
    ];

    // Available healthy candidates (failing node peer_b is excluded)
    let available_cluster_peers = vec![peer_a.to_string(), peer_c.to_string(), peer_d.to_string()];
    let existing_holders = vec![peer_a.to_string(), peer_c.to_string()];

    // 5. Generate repair plan enforcing failure domain placement
    let master_key = derive_master_key(passphrase, &salt).expect("derive master key");
    let file_key = derive_file_key(&master_key, &salt, b"finance.db");

    let plans = plan_encrypted_chunk_repair(
        &degraded[0],
        &surviving_inputs,
        &file_key,
        &available_cluster_peers,
        &existing_holders,
    )
    .expect("Plan chunk repair");

    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert_eq!(plan.shard_idx, 1);
    assert_eq!(plan.target_peer_id, peer_d.to_string());
    assert_eq!(plan.shard_hash, manifest.chunks[0].shard_hashes[1]);

    // 6. Node D receives and stores the reconstructed shard
    state_d.write_shard(&hash_1, &plan.shard_data).unwrap();

    // 7. Update file manifest with new holder
    tracked_manifest.chunks[0].shard_holders[1] = peer_d.to_string();

    // 8. Verify healthy manifest now after repair!
    let healthy_check = state_a.check_manifest_health(&tracked_manifest);
    assert!(
        healthy_check.is_empty(),
        "Chunk must be completely healthy after repair!"
    );

    // 9. Full decode roundtrip with reconstructed shard from Node D
    let reconstructed_file = decode_file(
        &tracked_manifest,
        passphrase,
        &salt,
        &[vec![
            Some(state_a.read_shard(&hash_0).unwrap()),
            Some(plan.shard_data.clone()),
            Some(state_c.read_shard(&hash_2).unwrap()),
        ]],
        k,
        m,
    )
    .expect("Decode file with repaired shard");

    assert_eq!(
        reconstructed_file, secret_file,
        "Decoded file after automated repair must be bit-for-bit identical to original!"
    );
}

#[test]
fn test_unrecoverable_boundary_when_below_k_shards() {
    let degraded = DegradedChunk::new(
        "unrecoverable-archive".into(),
        0,
        3,          // k = 3
        2,          // m = 2
        vec![0, 4], // Only 2 shards survive < k(3)
    );

    assert_eq!(degraded.missing_indices, vec![1, 2, 3]);
    assert!(!degraded.can_repair());
    assert_eq!(degraded.state(), RepairState::Unrecoverable);

    let surviving_inputs: Vec<Option<Vec<u8>>> =
        vec![Some(vec![1; 100]), None, None, None, Some(vec![2; 100])];
    let peers = vec!["peer-1".into(), "peer-2".into(), "peer-3".into()];
    let existing = vec!["peer-1".into()];

    let plan_res = plan_chunk_repair(&degraded, &surviving_inputs, &peers, &existing);
    assert_eq!(
        plan_res.unwrap_err(),
        RepairError::InsufficientShards {
            available: 2,
            required: 3
        }
    );
}
