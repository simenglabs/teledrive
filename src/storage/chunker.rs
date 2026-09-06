// 45 MB per chunk (MTProto supports up to 2GB per file)
pub const CHUNK_SIZE: usize = 45 * 1024 * 1024;

/// An internal byte range (end inclusive).
#[derive(Debug, Clone, Copy)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64, // inclusive
}

impl ByteRange {
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        (self.end - self.start + 1) as usize
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }
}

/// A planned sequence of Telegram chunk reads covering a byte range of an object.
#[derive(Debug, Clone)]
pub struct ChunkRead {
    /// Database chunk record id (part index into the ordered chunk list).
    pub chunk_index: usize,
    /// Byte offset inside the referenced Telegram file where the read starts.
    pub offset_in_chunk: u64,
    /// Number of bytes to read from that Telegram file.
    pub len: u64,
}

/// Resolve which Telegram chunks must be read (and from where) to serve
/// an object byte range without ever loading the whole object in memory.
pub fn resolve_range_reads(
    chunk_sizes: &[u64],
    range: ByteRange,
) -> Vec<ChunkRead> {
    let mut reads = Vec::new();
    let mut object_offset: u64 = 0;

    for (idx, &size) in chunk_sizes.iter().enumerate() {
        let chunk_start = object_offset;
        let chunk_end = object_offset + size; // exclusive

        // Intersection of [range.start, range.end] with this chunk
        let from = range.start.max(chunk_start);
        let to = (range.end + 1).min(chunk_end); // exclusive

        if to > from {
            reads.push(ChunkRead {
                chunk_index: idx,
                offset_in_chunk: from - chunk_start,
                len: to - from,
            });
        }

        object_offset = chunk_end;
        if object_offset > range.end {
            break;
        }
    }

    reads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_range_reads_single_chunk() {
        let sizes = vec![100u64];
        let reads = resolve_range_reads(&sizes, ByteRange { start: 10, end: 49 });
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].chunk_index, 0);
        assert_eq!(reads[0].offset_in_chunk, 10);
        assert_eq!(reads[0].len, 40);
    }

    #[test]
    fn test_resolve_range_reads_across_chunks() {
        let sizes = vec![50u64, 50, 50];
        let reads = resolve_range_reads(&sizes, ByteRange { start: 40, end: 119 });
        assert_eq!(reads.len(), 3);
        assert_eq!(reads[0].chunk_index, 0);
        assert_eq!(reads[0].offset_in_chunk, 40);
        assert_eq!(reads[0].len, 10);
        assert_eq!(reads[1].chunk_index, 1);
        assert_eq!(reads[1].offset_in_chunk, 0);
        assert_eq!(reads[1].len, 50);
        assert_eq!(reads[2].chunk_index, 2);
        assert_eq!(reads[2].offset_in_chunk, 0);
        assert_eq!(reads[2].len, 20);
    }

    #[test]
    fn test_resolve_range_reads_middle_chunk_only() {
        let sizes = vec![50u64, 50, 50];
        let reads = resolve_range_reads(&sizes, ByteRange { start: 60, end: 69 });
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].chunk_index, 1);
        assert_eq!(reads[0].offset_in_chunk, 10);
        assert_eq!(reads[0].len, 10);
    }
}
