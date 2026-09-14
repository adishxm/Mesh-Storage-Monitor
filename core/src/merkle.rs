use sha2::{Digest, Sha256};

pub type Hash256 = [u8; 32];

/// Computes the SHA-256 hash of the given data.
pub fn hash_data(data: &[u8]) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Computes the parent hash of a chunk based on its child shard hashes.
/// Consists of hashing the concatenation of all shard hashes.
pub fn compute_chunk_hash(shard_hashes: &[Hash256]) -> Hash256 {
    let mut hasher = Sha256::new();
    for hash in shard_hashes {
        hasher.update(hash);
    }
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Computes the root hash of a file based on its chunk hashes.
/// Consists of hashing the concatenation of all chunk hashes.
pub fn compute_root_hash(chunk_hashes: &[Hash256]) -> Hash256 {
    let mut hasher = Sha256::new();
    for hash in chunk_hashes {
        hasher.update(hash);
    }
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Verifies if a given shard matches its expected hash.
pub fn verify_shard(shard_data: &[u8], expected_hash: &Hash256) -> bool {
    let hash = hash_data(shard_data);
    &hash == expected_hash
}

/// Verifies if a list of shard hashes matches the expected chunk hash.
pub fn verify_chunk(shard_hashes: &[Hash256], expected_chunk_hash: &Hash256) -> bool {
    let computed = compute_chunk_hash(shard_hashes);
    &computed == expected_chunk_hash
}

/// Verifies if a list of chunk hashes matches the expected root hash.
pub fn verify_root(chunk_hashes: &[Hash256], expected_root_hash: &Hash256) -> bool {
    let computed = compute_root_hash(chunk_hashes);
    &computed == expected_root_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_data() {
        let data = b"hello world";
        let h = hash_data(data);
        // SHA-256 of "hello world"
        let expected = [
            185, 77, 39, 185, 147, 77, 62, 8, 165, 46, 82, 215, 218, 125, 171, 250, 196, 132, 239, 227, 122, 83, 128, 238, 144, 136, 247, 172, 226, 239, 205, 233]
        ;
        assert_eq!(h, expected);
    }

    #[test]
    fn test_merkle_verification() {
        let shard1 = b"shard-1-data";
        let shard2 = b"shard-2-data";

        let h1 = hash_data(shard1);
        let h2 = hash_data(shard2);

        // Verify shards
        assert!(verify_shard(shard1, &h1));
        assert!(verify_shard(shard2, &h2));
        assert!(!verify_shard(shard1, &h2));

        // Compute chunk hash
        let shard_hashes = vec![h1, h2];
        let chunk_hash = compute_chunk_hash(&shard_hashes);

        // Verify chunk hash
        assert!(verify_chunk(&shard_hashes, &chunk_hash));
        
        // Tampered shard hashes list
        let tampered_shard_hashes = vec![h1, h1];
        assert!(!verify_chunk(&tampered_shard_hashes, &chunk_hash));
    }
}
