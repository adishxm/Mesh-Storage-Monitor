use reed_solomon_erasure::galois_8::ReedSolomon;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ErasureError {
    #[error("Reed-Solomon error: {0}")]
    ReedSolomon(#[from] reed_solomon_erasure::Error),
    #[error("Invalid parameters: k={0}, m={1}")]
    InvalidParameters(usize, usize),
    #[error("Too few shards available: got {0}, need {1}")]
    TooFewShards(usize, usize),
}

/// Encodes data into `k` data shards and `m` parity shards.
/// Returns a list of `k + m` shards, where each shard is a `Vec<u8>`.
pub fn encode_data(data: &[u8], k: usize, m: usize) -> Result<Vec<Vec<u8>>, ErasureError> {
    if k == 0 || m == 0 {
        return Err(ErasureError::InvalidParameters(k, m));
    }

    let r = ReedSolomon::new(k, m)?;

    // Calculate shard size: round up chunk size divided by k
    let data_len = data.len();
    let shard_size = data_len.div_ceil(k);
    let padded_len = shard_size * k;

    // Create a padded buffer
    let mut padded_data = vec![0u8; padded_len];
    padded_data[..data_len].copy_from_slice(data);

    // Split padded data into k data shards
    let mut shards: Vec<Vec<u8>> = padded_data
        .chunks_exact(shard_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    // Append m parity shards (initialized to zero)
    for _ in 0..m {
        shards.push(vec![0u8; shard_size]);
    }

    // Perform Reed-Solomon encoding
    r.encode(&mut shards)?;

    Ok(shards)
}

/// Reconstructs the original data given a list of `k + m` shards (some of which may be `None`).
/// You must specify the original data length to truncate any padding.
pub fn reconstruct_data(
    shards: &[Option<Vec<u8>>],
    k: usize,
    m: usize,
    original_len: usize,
) -> Result<Vec<u8>, ErasureError> {
    if k == 0 {
        return Err(ErasureError::InvalidParameters(k, m));
    }

    // Count available shards
    let available = shards.iter().filter(|s| s.is_some()).count();
    if available < k {
        return Err(ErasureError::TooFewShards(available, k));
    }

    let r = ReedSolomon::new(k, m)?;

    // Prepare mutable option slice for reconstruction
    let mut mutable_shards = shards.to_vec();

    // Perform Reed-Solomon reconstruction
    r.reconstruct(&mut mutable_shards)?;

    // Gather reconstructed data shards (first k shards)
    let mut reconstructed_data = Vec::new();
    for shard in mutable_shards.iter().take(k) {
        let shard = shard
            .as_ref()
            .ok_or(reed_solomon_erasure::Error::TooFewShards)?;
        reconstructed_data.extend_from_slice(shard);
    }

    // Truncate any padding
    if reconstructed_data.len() > original_len {
        reconstructed_data.truncate(original_len);
    }

    Ok(reconstructed_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_erasure_roundtrip() {
        let original_data = b"This is some cool test data for Reed-Solomon erasure coding testing!";
        let k = 4;
        let m = 2;

        // Encode
        let shards = encode_data(original_data, k, m).unwrap();
        assert_eq!(shards.len(), k + m);

        // Turn shards into Option wrappers
        let mut opt_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();

        // Simulate losing m shards
        opt_shards[1] = None;
        opt_shards[3] = None;

        // Reconstruct
        let reconstructed = reconstruct_data(&opt_shards, k, m, original_data.len()).unwrap();
        assert_eq!(original_data.as_slice(), reconstructed.as_slice());
    }

    #[test]
    fn test_erasure_insufficient_shards_fails() {
        let original_data = b"Some data";
        let k = 3;
        let m = 2;

        let shards = encode_data(original_data, k, m).unwrap();
        let mut opt_shards: Vec<Option<Vec<u8>>> = shards.into_iter().map(Some).collect();

        // Simulate losing m + 1 shards (leaves less than k shards)
        opt_shards[0] = None;
        opt_shards[1] = None;
        opt_shards[2] = None;

        let result = reconstruct_data(&opt_shards, k, m, original_data.len());
        assert!(result.is_err());
    }
}
