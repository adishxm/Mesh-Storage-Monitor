use fastcdc::v2020::{FastCDC, StreamCDC};

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

/// Splits data from an arbitrary `std::io::Read` stream into chunks using FastCDC.
/// Operates in bounded memory without loading the entire stream into RAM.
pub fn chunk_stream<R: std::io::Read>(mut reader: R) -> Result<Vec<Vec<u8>>, std::io::Error> {
    let min_size = 524_288; // 512 KB
    let avg_size = 2_097_152; // 2 MB
    let max_size = 8_388_608; // 8 MB

    let chunker = StreamCDC::new(&mut reader, min_size, avg_size, max_size);
    let mut chunks = Vec::new();
    for entry in chunker {
        let chunk = entry?;
        chunks.push(chunk.data);
    }
    Ok(chunks)
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

    #[test]
    fn test_chunk_stream_empty() {
        let cursor = std::io::Cursor::new(vec![]);
        let chunks = chunk_stream(cursor).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_stream_small() {
        let data = vec![77u8; 1500];
        let cursor = std::io::Cursor::new(data.clone());
        let chunks = chunk_stream(cursor).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], data);
    }

    #[test]
    fn test_chunk_stream_equivalence_with_slice() {
        // Generate a 4MB dataset
        let mut data = vec![0u8; 4 * 1024 * 1024];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }

        let slice_chunks = chunk_data(&data);
        let stream_chunks = chunk_stream(std::io::Cursor::new(data.clone())).unwrap();

        assert_eq!(slice_chunks.len(), stream_chunks.len());
        for (a, b) in slice_chunks.iter().zip(stream_chunks.iter()) {
            assert_eq!(a, b);
        }
    }
}
