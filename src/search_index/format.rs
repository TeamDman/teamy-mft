use crate::domain::DriveLetter;
use crate::query::Pathlike;
use crate::search_index::SearchIndexFlags;
use std::io::Write;
use uom::si::information::byte;
use uom::si::usize::Information;

pub const SEARCH_INDEX_MAGIC: &[u8; 8] = b"TMFTIDX\0";
pub const SEARCH_INDEX_VERSION: u16 = 7;
pub const SEARCH_INDEX_HEADER_LEN: usize = 8 + 2 + 2 + 1 + 8 + 8;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SearchIndexPathRow {
    pub path: Pathlike,
    pub has_deleted_entries: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SearchIndexHeader {
    pub version: u16,
    pub flags: SearchIndexFlags,
    pub drive_letter: DriveLetter,
    pub source_mft_len: Information,
    pub node_count: u64,
}

impl SearchIndexHeader {
    #[must_use]
    pub fn new(drive_letter: DriveLetter, source_mft_len: Information, node_count: u64) -> Self {
        Self {
            version: SEARCH_INDEX_VERSION,
            flags: SearchIndexFlags::Reserved,
            drive_letter,
            source_mft_len,
            node_count,
        }
    }

    /// Write the serialized search index header bytes to a writer.
    ///
    /// # Errors
    ///
    /// Returns an error if the source length cannot fit the on-disk `u64`
    /// or the writer cannot accept the complete header.
    pub fn write_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
        let source_mft_len_bytes =
            u64::try_from(self.source_mft_len.get::<byte>()).map_err(std::io::Error::other)?;
        writer.write_all(SEARCH_INDEX_MAGIC)?;
        writer.write_all(&self.version.to_le_bytes())?;
        writer.write_all(&u16::from(self.flags).to_le_bytes())?;
        writer.write_all(&[u8::from(self.drive_letter)])?;
        writer.write_all(&source_mft_len_bytes.to_le_bytes())?;
        writer.write_all(&self.node_count.to_le_bytes())?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the source length cannot fit the on-disk `u64`.
    pub fn extend_vec(&self, bytes: &mut Vec<u8>) -> std::io::Result<()> {
        let source_mft_len_bytes =
            u64::try_from(self.source_mft_len.get::<byte>()).map_err(std::io::Error::other)?;
        bytes.extend_from_slice(SEARCH_INDEX_MAGIC);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&u16::from(self.flags).to_le_bytes());
        bytes.push(u8::from(self.drive_letter));
        bytes.extend_from_slice(&source_mft_len_bytes.to_le_bytes());
        bytes.extend_from_slice(&self.node_count.to_le_bytes());
        Ok(())
    }
}
