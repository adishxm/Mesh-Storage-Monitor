use mesh_core::{ChunkManifest, FileManifest, create_encrypted_backup, restore_encrypted_backup};
use mesh_node::api::{AppState, make_router};
use mesh_node::identity::load_or_create_keypair;
use mesh_node::state::NodeState;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::{RwLock, mpsc};

#[test]
fn test_multi_node_disaster_recovery_and_backup_restoration() {
    let temp_primary = tempdir().expect("primary tempdir");
    let primary_dir = temp_primary.path().to_path_buf();

    let primary_key = load_or_create_keypair(&primary_dir).expect("primary keypair");
    let primary_peer_id = libp2p::PeerId::from(primary_key.public());

    let mut primary_node = NodeState::with_data_dir(primary_peer_id, primary_dir, 5.0);
    primary_node.record_peer_storage("12D3KooWPeerAlice", 1048576, 524288);
    primary_node.record_peer_storage("12D3KooWPeerBob", 2097152, 0);

    let manifest = FileManifest {
        schema_version: 2,
        file_id: "cluster-mission-critical.db".to_string(),
        file_name: Some("mission.db".to_string()),
        original_len: 8192,
        root_hash: [42u8; 32],
        k: 2,
        m: 1,
        salt_hex: "deadbeef01020304".to_string(),
        created_at: 1728500000,
        chunks: vec![ChunkManifest {
            chunk_hash: [99u8; 32],
            shard_hashes: vec![[1u8; 32], [2u8; 32], [3u8; 32]],
            shard_holders: vec![],
            original_len: 8192,
        }],
    };
    primary_node
        .save_manifest(&manifest)
        .expect("save manifest");

    // 1. Create encrypted backup with master disaster recovery passphrase
    let passphrase = "enterprise_dr_passphrase_production_grade_secret";
    let snapshot = primary_node.create_backup_snapshot();
    let encrypted_archive =
        create_encrypted_backup(&snapshot, passphrase).expect("create encrypted backup");

    assert!(encrypted_archive.len() > 34 + 16);
    assert_eq!(&encrypted_archive[0..4], b"MBAK");

    // 2. Disaster scenario: Primary node crashes and directory is destroyed.
    drop(primary_node);
    drop(temp_primary);

    // 3. Replacement node boots up in a clean new environment.
    let temp_replacement = tempdir().expect("replacement tempdir");
    let replacement_dir = temp_replacement.path().to_path_buf();
    let replacement_key = load_or_create_keypair(&replacement_dir).expect("replacement keypair");
    let replacement_peer_id = libp2p::PeerId::from(replacement_key.public());

    let mut replacement_node = NodeState::with_data_dir(replacement_peer_id, replacement_dir, 0.5);

    // Initially replacement has empty state
    assert_eq!(replacement_node.list_manifests().len(), 0);
    assert_eq!(replacement_node.peer_credits.len(), 0);

    // 4. Restore state from encrypted backup archive
    let restored_snapshot =
        restore_encrypted_backup(&encrypted_archive, passphrase).expect("restore backup");
    let (restored_manifests, restored_peers) =
        replacement_node.restore_backup_snapshot(restored_snapshot);

    assert_eq!(restored_manifests, 1);
    assert_eq!(restored_peers, 2);
    assert_eq!(replacement_node.storage_quota, 5 * 1024 * 1024 * 1024);

    let restored_manifest = replacement_node
        .read_manifest("cluster-mission-critical.db")
        .expect("restored manifest must exist");
    assert_eq!(restored_manifest.root_hash, [42u8; 32]);
    assert_eq!(restored_manifest.k, 2);
    assert_eq!(restored_manifest.m, 1);

    assert!(
        replacement_node
            .peer_credits
            .contains_key("12D3KooWPeerAlice")
    );
    assert!(
        replacement_node
            .peer_credits
            .contains_key("12D3KooWPeerBob")
    );
}

#[tokio::test]
async fn test_prometheus_metrics_and_observability_pipeline() {
    let temp = tempdir().expect("metrics tempdir");
    let path = temp.path().to_path_buf();
    let key = load_or_create_keypair(&path).expect("keypair");
    let peer_id = libp2p::PeerId::from(key.public());

    let mut node = NodeState::with_data_dir(peer_id, path, 2.5);
    node.record_peer_storage("12D3KooWPeer1", 65536, 32768);

    let (tx, _rx) = mpsc::channel(1);
    let app_state = AppState {
        node_state: Arc::new(RwLock::new(node)),
        network_tx: tx,
    };

    let router = make_router(app_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/v1/metrics", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/plain"));

    let text = resp.text().await.unwrap();

    assert!(text.contains("mesh_storage_used_bytes"));
    assert!(text.contains("mesh_storage_quota_bytes"));
    assert!(text.contains("mesh_peers_connected"));
    assert!(text.contains("mesh_reciprocity_contributed_bytes"));
    assert!(text.contains("mesh_audit_challenges_passed_total"));
    assert!(text.contains("mesh_bandwidth_limit_kbps"));
}

#[test]
fn test_zero_leakage_and_secrets_redaction() {
    let secret_passphrase = "my_super_confidential_passphrase_98765";
    let snapshot =
        mesh_core::NodeBackupSnapshot::new("12D3KooWPeerTest".to_string(), 1000, 100, 1000);

    let archive = create_encrypted_backup(&snapshot, secret_passphrase).unwrap();

    // Verify raw archive does not contain the passphrase as cleartext anywhere
    let pass_bytes = secret_passphrase.as_bytes();
    let archive_contains_secret = archive
        .windows(pass_bytes.len())
        .any(|window| window == pass_bytes);
    assert!(
        !archive_contains_secret,
        "Encrypted archive must not contain plain passphrase bytes"
    );

    // Verify error outputs do not echo passwords
    let wrong_attempt = restore_encrypted_backup(&archive, "invalid_attempt_guess");
    assert!(wrong_attempt.is_err());
    let err_str = wrong_attempt.unwrap_err().to_string();
    assert!(
        !err_str.contains("invalid_attempt_guess"),
        "Error message must not reflect user passphrases"
    );
}

#[test]
fn test_tampered_backup_rejection_and_fail_closed() {
    let snapshot =
        mesh_core::NodeBackupSnapshot::new("12D3KooWPeerFailClosed".to_string(), 1000, 100, 1000);
    let mut archive = create_encrypted_backup(&snapshot, "correct_pass").unwrap();

    // Tamper with authentication tag at end
    let len = archive.len();
    archive[len - 1] ^= 0x01;

    let res = restore_encrypted_backup(&archive, "correct_pass");
    assert_eq!(res.unwrap_err(), mesh_core::BackupError::DecryptionFailed);
}
