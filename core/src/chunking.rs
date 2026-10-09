use fastcdc::v2020::FastCDC;

/// Splits data into chunks using FastCDC with ~2MB average chunk size.
/// Returns a list of chunks, where each chunk is a `Vec<u8>`.
pub fn chunk_data(data: &[u8]) -> Vec<Vec<u8>> {
    // Parameters for ~2MB average chunk size
    let min_size = 524_288; // 512 KB
    let avg_size = 2_097_152; // 2 MB
    let max_size = 8_388_608; // 8 MB

    let chunker = FastCDC::new(data, min_size, avg_size, max_size);
    let mut chunks = Vec::new();
    for chunk in chunker {
        let chunk_bytes = data[chunk.offset..(chunk.offset + chunk.length)].to_vec();
        chunks.push(chunk_bytes);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunking_empty() {
        let data = vec![];
        let chunks = chunk_data(&data);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunking_small() {
        let data = vec![42; 1000];
        let chunks = chunk_data(&data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 1000);
    }

    #[test]
    fn test_chunking_large() {
        // Generate 5MB of data
        let data = vec![0u8; 5 * 1024 * 1024];
        let chunks = chunk_data(&data);
        assert!(!chunks.is_empty());
        // Verify we can reassemble the data
        let reassembled: Vec<u8> = chunks.iter().flatten().copied().collect();
        assert_eq!(data, reassembled);
    }
}
