use eyre::Context;
use eyre::bail;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;

pub const SEARCH_INDEX_MAGIC: &[u8; 8] = b"TMFTIDX\0";
pub const SEARCH_INDEX_VERSION: u16 = 1;
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

    pub fn write_to_path(
        self,
        output_path: impl AsRef<Path>,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<()> {
        let output_path = output_path.as_ref();
        let temp_path = output_path.with_extension("mft_search_index.tmp");

        let file = std::fs::File::create(&temp_path).wrap_err_with(|| {
            format!(
                "Failed creating temporary search index file {}",
                temp_path.display()
            )
        })?;
        let mut writer = BufWriter::new(file);

        writer.write_all(SEARCH_INDEX_MAGIC)?;
        writer.write_all(&self.version.to_le_bytes())?;
        writer.write_all(&self.flags.to_le_bytes())?;
        writer.write_all(&self.drive_letter.to_le_bytes())?;
        writer.write_all(&self.source_mft_len_bytes.to_le_bytes())?;
        writer.write_all(&self.node_count.to_le_bytes())?;

        for row in rows {
            let path_bytes = row.path.as_bytes();
            let path_len: u32 = path_bytes.len().try_into().wrap_err_with(|| {
                format!("Path too long to encode in index row ({} bytes)", path_bytes.len())
            })?;
            writer.write_all(&path_len.to_le_bytes())?;
            writer.write_all(&[u8::from(row.has_deleted_entries)])?;
            writer.write_all(path_bytes)?;
        }

        writer.flush()?;

        std::fs::rename(&temp_path, output_path).wrap_err_with(|| {
            format!(
                "Failed atomically renaming search index file {} -> {}",
                temp_path.display(),
                output_path.display()
            )
        })?;

        Ok(())
    }

    pub fn parse(bytes: &[u8]) -> eyre::Result<Self> {
        if bytes.len() < SEARCH_INDEX_HEADER_LEN {
            bail!(
                "Invalid search index header: expected at least {} bytes, got {}",
                SEARCH_INDEX_HEADER_LEN,
                bytes.len()
            );
        }

        if &bytes[..SEARCH_INDEX_MAGIC.len()] != SEARCH_INDEX_MAGIC {
            bail!("Invalid search index header magic");
        }

        let mut cursor = SEARCH_INDEX_MAGIC.len();
        let read_u16 = |data: &[u8], cursor: &mut usize| -> u16 {
            let start = *cursor;
            let end = start + 2;
            *cursor = end;
            u16::from_le_bytes([data[start], data[start + 1]])
        };
        let read_u64 = |data: &[u8], cursor: &mut usize| -> u64 {
            let start = *cursor;
            let end = start + 8;
            *cursor = end;
            u64::from_le_bytes([
                data[start],
                data[start + 1],
                data[start + 2],
                data[start + 3],
                data[start + 4],
                data[start + 5],
                data[start + 6],
                data[start + 7],
            ])
        };

        let version = read_u16(bytes, &mut cursor);
        let flags = read_u16(bytes, &mut cursor);
        let drive_letter = bytes[cursor];
        cursor += 1;
        let source_mft_len_bytes = read_u64(bytes, &mut cursor);
        let node_count = read_u64(bytes, &mut cursor);

        Ok(Self {
            version,
            flags,
            drive_letter,
            source_mft_len_bytes,
            node_count,
        })
    }
}
