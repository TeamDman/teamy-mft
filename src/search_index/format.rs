use std::io::Write;

pub const SEARCH_INDEX_MAGIC: &[u8; 8] = b"TMFTIDX\0";
pub const SEARCH_INDEX_VERSION: u16 = 7;
pub const SEARCH_INDEX_HEADER_LEN: usize = 8 + 2 + 2 + 1 + 8 + 8;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SearchIndexPathRow {
    pub path: String,
    pub has_deleted_entries: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SearchIndexHeader {
    pub version: u16,
    pub flags: u16,
    pub drive_letter: u8,
    pub source_mft_len_bytes: u64,
    pub node_count: u64,
}

impl SearchIndexHeader {
    #[must_use]
    pub fn new(drive_letter: char, source_mft_len_bytes: u64, node_count: u64) -> Self {
        Self {
            version: SEARCH_INDEX_VERSION,
            flags: 0,
            drive_letter: drive_letter as u8,
            source_mft_len_bytes,
            node_count,
        }
    }

    /// Write the serialized search index header bytes to a writer.
    ///
    /// # Errors
    ///
    /// Returns an error if the writer cannot accept the complete header.
    pub fn write_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
        writer.write_all(SEARCH_INDEX_MAGIC)?;
        writer.write_all(&self.version.to_le_bytes())?;
        writer.write_all(&self.flags.to_le_bytes())?;
        writer.write_all(&self.drive_letter.to_le_bytes())?;
        writer.write_all(&self.source_mft_len_bytes.to_le_bytes())?;
        writer.write_all(&self.node_count.to_le_bytes())?;
        Ok(())
    }

    pub fn extend_vec(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(SEARCH_INDEX_MAGIC);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.flags.to_le_bytes());
        bytes.extend_from_slice(&self.drive_letter.to_le_bytes());
        bytes.extend_from_slice(&self.source_mft_len_bytes.to_le_bytes());
        bytes.extend_from_slice(&self.node_count.to_le_bytes());
    }
}
