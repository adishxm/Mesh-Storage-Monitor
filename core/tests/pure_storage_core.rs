use mesh_core::{
    chunking::{chunk_data, chunk_stream},
    crypto::{derive_file_key, derive_master_key, derive_shard_iv, encrypt_data},
    decode_file, encode_file, encode_reader,
    erasure::encode_data,
    format::{ShardHeader, pack_shard, unpack_shard},
    merkle::hash_data,
    quota::{QuotaPolicy, QuotaTracker},
};

#[test]
fn test_edge_case_empty_file() {
    let empty_data = b"";
    let passphrase = b"strong_password_123";
    let salt = b"salt_for_empty_file";
    let file_id = "empty-file-uuid";
    let k = 2;
    let m = 1;

    let (manifest, encoded_chunks) =
        encode_file(empty_data, passphrase, salt, file_id, k, m).expect("encode empty succeeds");

    assert_eq!(manifest.original_len, 0);
    assert_eq!(manifest.chunks.len(), 0);
    assert_eq!(encoded_chunks.len(), 0);

    let empty_shards: Vec<Vec<Option<Vec<u8>>>> = Vec::new();
    let decoded =
        decode_file(&manifest, passphrase, salt, &empty_shards, k, m).expect("decode succeeds");
    assert_eq!(decoded, empty_data);
}

#[test]
fn test_edge_case_single_byte_file() {
    let single_byte = b"X";
    let passphrase = b"passphrase_single_byte";
    let salt = b"salt_single_byte";
    let file_id = "single-byte-uuid";
    let k = 2;
    let m = 1;

    let (manifest, encoded_chunks) =
        encode_file(single_byte, passphrase, salt, file_id, k, m).expect("encode succeeds");

    let shards_present: Vec<Vec<Option<Vec<u8>>>> = encoded_chunks
        .iter()
        .map(|chunk| chunk.iter().map(|s| Some(s.clone())).collect())
        .collect();

    let decoded =
        decode_file(&manifest, passphrase, salt, &shards_present, k, m).expect("decode succeeds");
    assert_eq!(decoded, single_byte);
}

#[test]
fn test_packed_shard_pipeline_integration() {
    let file_data = b"This is a high-value dataset for container packaging tests.";
    let passphrase = b"container_password_123";
    let salt = b"container_salt_123";
    let file_id = "packed-shard-test-id";
    let k = 2;
    let m = 1;

    let (manifest, encoded_chunks) =
        encode_file(file_data, passphrase, salt, file_id, k, m).expect("encode succeeds");

    // Pack each shard into a versioned binary container on "disk"
    let mut stored_containers = Vec::new();
    for (chunk_idx, chunk_shards) in encoded_chunks.iter().enumerate() {
        let mut chunk_containers = Vec::new();
        for (shard_idx, shard_bytes) in chunk_shards.iter().enumerate() {
            let shard_hash = manifest.chunks[chunk_idx].shard_hashes[shard_idx];
            let header = ShardHeader::new(
                file_id.to_string(),
                chunk_idx,
                shard_idx,
                shard_hash,
                k,
                m,
                shard_bytes.len(),
                1775779200,
            );
            let container = pack_shard(&header, shard_bytes).expect("pack shard succeeds");
            chunk_containers.push(container);
        }
        stored_containers.push(chunk_containers);
    }

    // Now unpack each container, verifying integrity before decode
    let mut retrieved_shards: Vec<Vec<Option<Vec<u8>>>> = Vec::new();
    for chunk_containers in stored_containers {
        let mut chunk_shards = Vec::new();
        for container in chunk_containers {
            let unpacked = unpack_shard(&container).expect("unpack and verify succeeds");
            chunk_shards.push(Some(unpacked.payload));
        }
        retrieved_shards.push(chunk_shards);
    }

    let decoded =
        decode_file(&manifest, passphrase, salt, &retrieved_shards, k, m).expect("decode succeeds");
    assert_eq!(decoded, file_data);
}

#[test]
fn test_deterministic_repair_hash_identity() {
    let original_data =
        b"Deterministic IV property test dataset. Ensure re-encryption hash matches.";
    let passphrase = b"deterministic_pw_99";
    let salt = b"deterministic_salt_99";
    let file_id = "repair-hash-match-id";
    let k = 2;
    let m = 1;

    let (manifest, encoded_chunks) =
        encode_file(original_data, passphrase, salt, file_id, k, m).expect("encode succeeds");

    let chunk_0_shards = &encoded_chunks[0];
    let original_shard_0_hash = manifest.chunks[0].shard_hashes[0];

    // Compute plaintext shard 0 directly
    let plain_chunks = chunk_data(original_data);
    let plain_shards = encode_data(&plain_chunks[0], k, m).expect("rs encode succeeds");
    let plain_shard_0 = &plain_shards[0];

    // Re-encrypt shard 0 using deterministic IV
    let master_key = derive_master_key(passphrase, salt).expect("key derivation succeeds");
    let file_key = derive_file_key(&master_key, salt, file_id.as_bytes());
    let shard_0_iv = derive_shard_iv(&file_key, 0, 0);
    let re_encrypted_shard_0 =
        encrypt_data(plain_shard_0, &file_key, &shard_0_iv).expect("re-encrypt succeeds");

    // Verify hash of re-encrypted shard is 100% IDENTICAL to manifest recorded hash
    let computed_hash = hash_data(&re_encrypted_shard_0);
    assert_eq!(computed_hash, original_shard_0_hash);
    assert_eq!(re_encrypted_shard_0, chunk_0_shards[0]);
}

#[test]
fn test_wrong_passphrase_fails_decryption() {
    let data = b"Confidential financial records";
    let (manifest, encoded_chunks) =
        encode_file(data, b"correct_password", b"salt_correct", "f-auth", 2, 1).unwrap();

    let shards: Vec<Vec<Option<Vec<u8>>>> = encoded_chunks
        .into_iter()
        .map(|c| c.into_iter().map(Some).collect())
        .collect();

    let err =
        decode_file(&manifest, b"wrong_password", b"salt_correct", &shards, 2, 1).unwrap_err();
    assert!(err.contains("AES-GCM encryption/decryption error"));
}

#[test]
fn test_wrong_salt_fails_decryption() {
    let data = b"Confidential medical records";
    let (manifest, encoded_chunks) =
        encode_file(data, b"correct_password", b"salt_correct_1", "f-salt", 2, 1).unwrap();

    let shards: Vec<Vec<Option<Vec<u8>>>> = encoded_chunks
        .into_iter()
        .map(|c| c.into_iter().map(Some).collect())
        .collect();

    let err = decode_file(
        &manifest,
        b"correct_password",
        b"salt_wrong_different",
        &shards,
        2,
        1,
    )
    .unwrap_err();
    assert!(err.contains("AES-GCM encryption/decryption error"));
}

#[test]
fn test_insufficient_shards_fails_cleanly() {
    let data = b"Important resilient document";
    let (manifest, encoded_chunks) = encode_file(
        data,
        b"password_resilient",
        b"salt_resilient",
        "f-res",
        3,
        2,
    )
    .unwrap();

    // With k=3, m=2, total shards is 5. If we lose 3 shards, only 2 remain (less than k=3).
    let mut shards: Vec<Vec<Option<Vec<u8>>>> = encoded_chunks
        .into_iter()
        .map(|c| c.into_iter().map(Some).collect())
        .collect();

    shards[0][0] = None;
    shards[0][1] = None;
    shards[0][2] = None; // 3 missing

    let err = decode_file(
        &manifest,
        b"password_resilient",
        b"salt_resilient",
        &shards,
        3,
        2,
    )
    .unwrap_err();
    assert!(err.contains("Cannot reconstruct chunk 0: only 2 valid shards available"));
}

#[test]
fn test_quota_tracker_integration() {
    let mut tracker =
        QuotaTracker::from_policy(500_000_000, &QuotaPolicy::Percentage(1.0)).unwrap();
    assert_eq!(tracker.quota_bytes, 5_000_000);

    // Store a 2MB shard
    tracker.record_store(2_000_000).unwrap();
    assert_eq!(tracker.used_bytes, 2_000_000);
    assert_eq!(tracker.remaining_bytes(), 3_000_000);

    // Another 2MB shard
    tracker.record_store(2_000_000).unwrap();
    assert_eq!(tracker.used_bytes, 4_000_000);

    // 2MB shard fails because only 1MB remains
    assert!(!tracker.can_store(2_000_000));
    assert!(tracker.record_store(2_000_000).is_err());
}

#[test]
fn test_streaming_chunker_bounded_memory() {
    // 5 MB synthetic stream
    let mut raw_data = vec![0u8; 5 * 1024 * 1024];
    for (i, byte) in raw_data.iter_mut().enumerate() {
        *byte = (i % 239) as u8;
    }

    let cursor = std::io::Cursor::new(raw_data.clone());
    let chunks = chunk_stream(cursor).expect("chunk_stream succeeds");

    assert!(!chunks.is_empty());
    // Verify bounded chunk sizes
    for chunk in &chunks {
        assert!(chunk.len() >= 524_288 || chunks.len() == 1);
        assert!(chunk.len() <= 8_388_608);
    }

    // Verify exact reconstruction
    let reassembled: Vec<u8> = chunks.into_iter().flatten().collect();
    assert_eq!(reassembled, raw_data);
}

#[test]
fn test_encode_reader_streaming_roundtrip() {
    let mut large_stream_data = vec![0u8; 3 * 1024 * 1024];
    for (i, b) in large_stream_data.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let passphrase = b"streaming_pass_999";
    let salt = b"streaming_salt_999";
    let file_id = "streaming-file-uuid";
    let k = 2;
    let m = 1;

    let cursor = std::io::Cursor::new(large_stream_data.clone());
    let (manifest, encoded_chunks) =
        encode_reader(cursor, passphrase, salt, file_id, k, m).expect("encode_reader succeeds");

    assert_eq!(manifest.original_len, large_stream_data.len());
    assert!(!manifest.chunks.is_empty());

    let shards_present: Vec<Vec<Option<Vec<u8>>>> = encoded_chunks
        .iter()
        .map(|chunk| chunk.iter().map(|s| Some(s.clone())).collect())
        .collect();

    let decoded =
        decode_file(&manifest, passphrase, salt, &shards_present, k, m).expect("decode succeeds");
    assert_eq!(decoded, large_stream_data);
}
