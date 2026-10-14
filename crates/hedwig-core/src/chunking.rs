use fastcdc::v2020::FastCDC;
use serde::{Deserialize, Serialize};

pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
pub const AVG_CHUNK_SIZE: u32 = 256 * 1024;
pub const MAX_CHUNK_SIZE: u32 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkFormat {
    V1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkDescriptor {
    pub hash: String,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub descriptor: ChunkDescriptor,
    pub offset: usize,
    pub data: &'a [u8],
}

pub fn chunk_bytes(data: &[u8], format: ChunkFormat) -> Vec<Chunk<'_>> {
    if data.is_empty() {
        return Vec::new();
    }

    let (minimum, average, maximum) = match format {
        ChunkFormat::V1 => (MIN_CHUNK_SIZE, AVG_CHUNK_SIZE, MAX_CHUNK_SIZE),
    };

    FastCDC::new(data, minimum, average, maximum)
        .map(|entry| {
            let bytes = &data[entry.offset..entry.offset + entry.length];
            Chunk {
                descriptor: ChunkDescriptor {
                    hash: blake3::hash(bytes).to_hex().to_string(),
                    size: entry.length as u32,
                },
                offset: entry.offset,
                data: bytes,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_needs_no_blocks() {
        assert!(chunk_bytes(&[], ChunkFormat::V1).is_empty());
    }

    #[test]
    fn chunks_reconstruct_original_content() {
        let data: Vec<u8> = (0..2_000_000).map(|i| ((i * 31) % 251) as u8).collect();
        let chunks = chunk_bytes(&data, ChunkFormat::V1);
        assert!(!chunks.is_empty());
        let reconstructed: Vec<u8> = chunks
            .iter()
            .flat_map(|chunk| chunk.data)
            .copied()
            .collect();
        assert_eq!(reconstructed, data);
        assert!(
            chunks.iter().all(|chunk| {
                chunk.descriptor.hash == blake3::hash(chunk.data).to_hex().as_str()
            })
        );
    }

    #[test]
    fn local_edit_reuses_most_large_file_blocks() {
        let mut before = vec![0_u8; 8 * 1024 * 1024];
        for (index, byte) in before.iter_mut().enumerate() {
            *byte = ((index * 97 + index / 4096) % 251) as u8;
        }
        let mut after = before.clone();
        after[4_000_000..4_001_000].fill(42);

        let before_hashes: std::collections::HashSet<_> = chunk_bytes(&before, ChunkFormat::V1)
            .into_iter()
            .map(|chunk| chunk.descriptor.hash)
            .collect();
        let after_chunks = chunk_bytes(&after, ChunkFormat::V1);
        let reused_bytes: usize = after_chunks
            .iter()
            .filter(|chunk| before_hashes.contains(&chunk.descriptor.hash))
            .map(|chunk| chunk.data.len())
            .sum();

        assert!(reused_bytes > before.len() * 3 / 4);
    }
}
