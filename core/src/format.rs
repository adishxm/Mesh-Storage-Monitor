use crate::merkle::{Hash256, hash_data};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SHARD_MAGIC: &[u8; 4] = b"MSHS";
pub const CURRENT_FORMAT_VERSION: u16 = 1;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum FormatError {
    #[error("Data too short: expected at least {0} bytes, got {1}")]
    TooShort(usize, usize),

    #[error("Invalid magic bytes: expected {:?}, got {:?}", SHARD_MAGIC, .0)]
    InvalidMagic([u8; 4]),

    #[error("Unsupported format version: {0} (current is {CURRENT_FORMAT_VERSION})")]
    UnsupportedVersion(u16),

    #[error("Invalid header length")]
    InvalidHeaderLength,

    #[error("Failed to parse shard header JSON: {0}")]
    InvalidHeaderJson(String),

    #[error("Payload length mismatch: header says {expected} bytes, but got {actual}")]
    PayloadLengthMismatch { expected: usize, actual: usize },

    #[error("Payload hash verification failed")]
    CorruptPayloadHash,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ShardHeader {
    pub format_version: u16,
    pub file_id: String,
    pub chunk_idx: usize,
    pub shard_idx: usize,
    pub shard_hash: Hash256,
    pub k: usize,
    pub m: usize,
    pub payload_len: usize,
    pub created_at: u64,
}

impl ShardHeader {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        file_id: String,
        chunk_idx: usize,
        shard_idx: usize,
        shard_hash: Hash256,
        k: usize,
        m: usize,
        payload_len: usize,
        created_at: u64,
    ) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            file_id,
            chunk_idx,
            shard_idx,
            shard_hash,
            k,
            m,
            payload_len,
            created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedShard {
    pub header: ShardHeader,
    pub payload: Vec<u8>,
}

/// Packs a shard header and its encrypted payload into a self-describing binary container.
///
/// Binary layout:
/// - 4 bytes: Magic `b"MSHS"`
/// - 2 bytes: Version (u16, Big Endian)
/// - 2 bytes: Header JSON byte length (u16, Big Endian)
/// - N bytes: Header JSON UTF-8
/// - M bytes: Encrypted shard payload
pub fn pack_shard(header: &ShardHeader, payload: &[u8]) -> Result<Vec<u8>, FormatError> {
    let mut cloned_header = header.clone();
    cloned_header.payload_len = payload.len();

    let header_json = serde_json::to_vec(&cloned_header)
        .map_err(|e| FormatError::InvalidHeaderJson(e.to_string()))?;

    let header_len =
        u16::try_from(header_json.len()).map_err(|_| FormatError::InvalidHeaderLength)?;

    let total_len = 4 + 2 + 2 + header_json.len() + payload.len();
    let mut container = Vec::with_capacity(total_len);

    container.extend_from_slice(SHARD_MAGIC);
    container.extend_from_slice(&cloned_header.format_version.to_be_bytes());
    container.extend_from_slice(&header_len.to_be_bytes());
    container.extend_from_slice(&header_json);
    container.extend_from_slice(payload);

    Ok(container)
}

/// Unpacks and validates a binary shard container.
/// Verifies magic bytes, supported version, header length, JSON integrity,
/// payload length match, and payload hash matching `header.shard_hash`.
pub fn unpack_shard(data: &[u8]) -> Result<PackedShard, FormatError> {
    const MIN_CONTAINER_SIZE: usize = 4 + 2 + 2; // Magic + Version + HeaderLen
    if data.len() < MIN_CONTAINER_SIZE {
        return Err(FormatError::TooShort(MIN_CONTAINER_SIZE, data.len()));
    }

    let mut magic = [0u8; 4];
    magic.copy_from_slice(&data[0..4]);
    if &magic != SHARD_MAGIC {
        return Err(FormatError::InvalidMagic(magic));
    }

    let version = u16::from_be_bytes([data[4], data[5]]);
    if version != CURRENT_FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }

    let header_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let header_end = MIN_CONTAINER_SIZE + header_len;

    if data.len() < header_end {
        return Err(FormatError::TooShort(header_end, data.len()));
    }

    let header_bytes = &data[MIN_CONTAINER_SIZE..header_end];
    let header: ShardHeader = serde_json::from_slice(header_bytes)
        .map_err(|e| FormatError::InvalidHeaderJson(e.to_string()))?;

    let payload = data[header_end..].to_vec();

    if payload.len() != header.payload_len {
        return Err(FormatError::PayloadLengthMismatch {
            expected: header.payload_len,
            actual: payload.len(),
        });
    }

    let computed_hash = hash_data(&payload);
    if computed_hash != header.shard_hash {
        return Err(FormatError::CorruptPayloadHash);
    }

    Ok(PackedShard { header, payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_unpack_roundtrip() {
        let payload = b"encrypted_shard_content_sample_bytes_12345";
        let shard_hash = hash_data(payload);

        let header = ShardHeader::new(
            "file_uuid_001".to_string(),
            0,
            1,
            shard_hash,
            2,
            1,
            payload.len(),
            1775779200,
        );

        let packed = pack_shard(&header, payload).expect("packing succeeds");
        assert!(packed.starts_with(b"MSHS"));

        let unpacked = unpack_shard(&packed).expect("unpacking succeeds");
        assert_eq!(unpacked.header.file_id, "file_uuid_001");
        assert_eq!(unpacked.header.chunk_idx, 0);
        assert_eq!(unpacked.header.shard_idx, 1);
        assert_eq!(unpacked.header.k, 2);
        assert_eq!(unpacked.header.m, 1);
        assert_eq!(unpacked.header.shard_hash, shard_hash);
        assert_eq!(unpacked.payload, payload);
    }

    #[test]
    fn test_unpack_too_short() {
        let err = unpack_shard(b"MSH").unwrap_err();
        assert!(matches!(err, FormatError::TooShort(8, 3)));
    }

    #[test]
    fn test_unpack_invalid_magic() {
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(b"NOPE");
        let err = unpack_shard(&data).unwrap_err();
        assert_eq!(err, FormatError::InvalidMagic(*b"NOPE"));
    }

    #[test]
    fn test_unpack_unsupported_version() {
        let mut data = vec![0u8; 16];
        data[0..4].copy_from_slice(SHARD_MAGIC);
        data[4..6].copy_from_slice(&99u16.to_be_bytes());
        let err = unpack_shard(&data).unwrap_err();
        assert_eq!(err, FormatError::UnsupportedVersion(99));
    }

    #[test]
    fn test_unpack_corrupt_payload_hash() {
        let payload = b"good_payload";
        let shard_hash = hash_data(payload);
        let header = ShardHeader::new(
            "f1".to_string(),
            0,
            0,
            shard_hash,
            1,
            1,
            payload.len(),
            1000,
        );

        let mut packed = pack_shard(&header, payload).unwrap();
        // Tamper with the last byte of the payload
        let last = packed.len() - 1;
        packed[last] ^= 0xFF;

        let err = unpack_shard(&packed).unwrap_err();
        assert_eq!(err, FormatError::CorruptPayloadHash);
    }
}
