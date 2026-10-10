use mesh_core::{CapacityVerificationGuard, ReciprocityTier, SubnetDensityGuard, SybilError};
use mesh_node::state::NodeState;
use tempfile::tempdir;

#[test]
fn test_credit_allowance_accrual_from_verified_storage() {
    let tmp = tempdir().expect("tempdir");
    let key = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = libp2p::PeerId::from(key.public());
    let mut state = NodeState::with_data_dir(peer_id, tmp.path().to_path_buf(), 5.0);

    // Initial state: 0 bytes contributed, 0 consumed
    let base_free = 1_073_741_824; // 1 GB free
    let ratio = 1.0;
    let cap = state.storage_quota * 2;
    assert_eq!(
        state.local_credits.evaluate_tier(base_free, ratio, cap),
        ReciprocityTier::Probationary
    );

    // Write real shards contributing storage
    let shard_data = vec![7u8; 1024 * 1024]; // 1 MB shard
    let shard_hash = hex::encode(mesh_core::merkle::hash_data(&shard_data));
    state
        .write_shard(&shard_hash, &shard_data)
        .expect("write shard");

    assert_eq!(state.local_credits.bytes_contributed, 1024 * 1024);

    // Pass 5 audits
    for i in 1..=5 {
        state.local_credits.record_audit(true);
        state.local_credits.record_uptime(3600 * i);
    }

    assert_eq!(
        state.local_credits.evaluate_tier(base_free, ratio, cap),
        ReciprocityTier::Contributor
    );
    assert!(
        state
            .local_credits
            .earned_allowance_bytes(base_free, ratio, cap)
            >= base_free
    );
    assert_eq!(state.local_credits.fair_share_ratio(), 2.0); // 0 consumed, >0 contributed
}

#[test]
fn test_freerider_overdraft_clamps_service_tier() {
    let tmp = tempdir().expect("tempdir");
    let key = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = libp2p::PeerId::from(key.public());
    let mut state = NodeState::with_data_dir(peer_id, tmp.path().to_path_buf(), 5.0);

    let untrusted_peer = "12D3KooWFreeRiderSybilPeer123456789";

    // Remote peer consumes 2 GB storage without contributing anything
    state.record_peer_storage(untrusted_peer, 0, 2_147_483_648);

    assert!(state.is_peer_throttled(untrusted_peer));

    let peer_ledger = state.peer_credits.get(untrusted_peer).unwrap();
    let base_free = 1_073_741_824;
    let ratio = 1.0;
    let cap = 50_000_000_000;
    assert_eq!(
        peer_ledger.evaluate_tier(base_free, ratio, cap),
        ReciprocityTier::Throttled
    );
    assert!(peer_ledger.credit_balance(base_free, ratio, cap) < 0);

    // Remote peer continues consuming 5 GB more -> Suspended
    state.record_peer_storage(untrusted_peer, 0, 5_000_000_000);
    let peer_ledger = state.peer_credits.get(untrusted_peer).unwrap();
    assert_eq!(
        peer_ledger.evaluate_tier(base_free, ratio, cap),
        ReciprocityTier::Suspended
    );
}

#[test]
fn test_subnet_density_guard_blocks_sybil_swarm() {
    let mut guard = SubnetDensityGuard::new(3); // Cap of 3 devices per /24 subnet

    let home_subnet = "192.168.1.0/24";
    let office_subnet = "10.0.0.0/24";

    assert!(guard.register_peer(home_subnet, "device-1").is_ok());
    assert!(guard.register_peer(home_subnet, "device-2").is_ok());
    assert!(guard.register_peer(home_subnet, "device-3").is_ok());

    // 4th Sybil device on home subnet is blocked!
    let err = guard
        .register_peer(home_subnet, "sybil-clone-4")
        .unwrap_err();
    assert_eq!(
        err,
        SybilError::SubnetDensityExceeded {
            subnet: home_subnet.to_string(),
            max_allowed: 3,
        }
    );

    // Independent office subnet can still enroll devices
    assert!(guard.register_peer(office_subnet, "office-node-1").is_ok());
    assert_eq!(guard.subnet_count(office_subnet), 1);
    assert_eq!(guard.subnet_count(home_subnet), 3);
}

#[test]
fn test_fake_capacity_claim_rejected_without_proof() {
    let guard = CapacityVerificationGuard::new(10); // 10 audits per GB

    let claimed_50gb = 50 * 1_073_741_824;

    // Zero audits -> Rejected
    assert!(guard.verify_capacity_claim(claimed_50gb, 0).is_err());

    // 200 audits (less than required 500) -> Rejected
    let err = guard.verify_capacity_claim(claimed_50gb, 200).unwrap_err();
    assert_eq!(
        err,
        SybilError::UnverifiedCapacityClaim {
            claimed_bytes: claimed_50gb,
            required_audits: 500,
            actual_audits: 200,
        }
    );

    // 500 audits -> Verified!
    let verified = guard.verify_capacity_claim(claimed_50gb, 500).unwrap();
    assert_eq!(verified, claimed_50gb);
}
