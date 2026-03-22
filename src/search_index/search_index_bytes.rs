use crate::search_index::format::SEARCH_INDEX_HEADER_LEN;
use crate::search_index::format::SEARCH_INDEX_MAGIC;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use eyre::Context;
use eyre::bail;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexBytes<'a> {
    bytes: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct SearchIndexBytesMut {
    header: SearchIndexHeader,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexPathRowView<'a> {
    pub path: &'a str,
    pub normalized_path: Option<&'a str>,
    pub has_deleted_entries: bool,
}

impl SearchIndexPathRowView<'_> {
    #[must_use]
    pub fn to_owned(self) -> SearchIndexPathRow {
        SearchIndexPathRow {
            path: self.path.to_owned(),
            has_deleted_entries: self.has_deleted_entries,
        }
    }
}

#[derive(Debug)]
pub struct SearchIndexRowIter<'a> {
    bytes: &'a [u8],
    cursor: usize,
    remaining_rows: usize,
    row_index: usize,
}

impl<'a> SearchIndexBytes<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// # Errors
    ///
    /// Returns an error if the bytes are too short to contain a full header or
    /// if the header magic does not match the expected search index magic.
    pub fn header(&self) -> eyre::Result<SearchIndexHeader> {
        if self.bytes.len() < SEARCH_INDEX_HEADER_LEN {
            bail!(
                "Invalid search index header: expected at least {} bytes, got {}",
                SEARCH_INDEX_HEADER_LEN,
                self.bytes.len()
            );
        }

        if &self.bytes[..SEARCH_INDEX_MAGIC.len()] != SEARCH_INDEX_MAGIC {
            bail!("Invalid search index header magic");
        }

        let mut cursor = SEARCH_INDEX_MAGIC.len();
        let version = self.read_u16(&mut cursor);
        let flags = self.read_u16(&mut cursor);
        let drive_letter = self.bytes[cursor];
        cursor += 1;
        let source_mft_len_bytes = self.read_u64(&mut cursor);
        let node_count = self.read_u64(&mut cursor);

        Ok(SearchIndexHeader {
            version,
            flags,
            drive_letter,
            source_mft_len_bytes,
            node_count,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the header cannot be parsed.
    pub fn version(&self) -> eyre::Result<u16> {
        Ok(self.header()?.version)
    }

    /// # Errors
    ///
    /// Returns an error if the header cannot be parsed or if the stored node
    /// count does not fit in `usize`.
    pub fn row_views(&self) -> eyre::Result<SearchIndexRowIter<'a>> {
        let header = self.header()?;
        let node_count = usize::try_from(header.node_count).wrap_err_with(|| {
            format!(
                "Search index node count {} does not fit into usize",
                header.node_count
            )
        })?;

        Ok(SearchIndexRowIter {
            bytes: self.bytes,
            cursor: SEARCH_INDEX_HEADER_LEN,
            remaining_rows: node_count,
            row_index: 0,
        })
    }

    fn read_u16(&self, cursor: &mut usize) -> u16 {
        let start = *cursor;
        let end = start + 2;
        *cursor = end;
        u16::from_le_bytes([self.bytes[start], self.bytes[start + 1]])
    }

    fn read_u64(&self, cursor: &mut usize) -> u64 {
        let start = *cursor;
        let end = start + 8;
        *cursor = end;
        u64::from_le_bytes([
            self.bytes[start],
            self.bytes[start + 1],
            self.bytes[start + 2],
            self.bytes[start + 3],
            self.bytes[start + 4],
            self.bytes[start + 5],
            self.bytes[start + 6],
            self.bytes[start + 7],
        ])
    }
}

impl<'a> Iterator for SearchIndexRowIter<'a> {
    type Item = eyre::Result<SearchIndexPathRowView<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_rows == 0 {
            return None;
        }

        let row_index = self.row_index;
        self.row_index += 1;
        self.remaining_rows -= 1;

        let row_header_len = 4 + 4 + 1;
        if self.bytes.len() < self.cursor + row_header_len {
            return Some(Err(eyre::eyre!(
                "Corrupt search index: truncated row header at row {}",
                row_index
            )));
        }

        let path_len_start = self.cursor;
        self.cursor += 4;
        let path_len = u32::from_le_bytes([
            self.bytes[path_len_start],
            self.bytes[path_len_start + 1],
            self.bytes[path_len_start + 2],
            self.bytes[path_len_start + 3],
        ]) as usize;

        let normalized_path_len_start = self.cursor;
        self.cursor += 4;
        let normalized_path_len = u32::from_le_bytes([
            self.bytes[normalized_path_len_start],
            self.bytes[normalized_path_len_start + 1],
            self.bytes[normalized_path_len_start + 2],
            self.bytes[normalized_path_len_start + 3],
        ]) as usize;

        let has_deleted_entries = self.bytes[self.cursor] != 0;
        self.cursor += 1;

        let path_end = self.cursor + path_len;
        if self.bytes.len() < path_end {
            return Some(Err(eyre::eyre!(
                "Corrupt search index: truncated path payload at row {}",
                row_index
            )));
        }

        let path = match std::str::from_utf8(&self.bytes[self.cursor..path_end]) {
            Ok(path) => path,
            Err(error) => {
                return Some(
                    Err(error)
                        .wrap_err_with(|| format!("Invalid UTF-8 path payload at row {row_index}")),
                );
            }
        };

        let normalized_path_end = path_end + normalized_path_len;
        if self.bytes.len() < normalized_path_end {
            return Some(Err(eyre::eyre!(
                "Corrupt search index: truncated normalized path payload at row {}",
                row_index
            )));
        }

        let normalized_path = match std::str::from_utf8(&self.bytes[path_end..normalized_path_end])
        {
            Ok(path) => path,
            Err(error) => {
                return Some(Err(error).wrap_err_with(|| {
                    format!("Invalid UTF-8 normalized path payload at row {row_index}")
                }));
            }
        };

        self.cursor = normalized_path_end;

        Some(Ok(SearchIndexPathRowView {
            path,
            normalized_path: Some(normalized_path),
            has_deleted_entries,
        }))
    }
}

impl SearchIndexBytesMut {
    #[must_use]
    pub fn new(header: SearchIndexHeader) -> Self {
        let mut bytes = Vec::with_capacity(SEARCH_INDEX_HEADER_LEN);
        header.extend_vec(&mut bytes);

        Self { header, bytes }
    }

    /// # Errors
    ///
    /// Returns an error if the row path or normalized path is too long to encode.
    pub fn push_row(&mut self, row: &SearchIndexPathRow) -> eyre::Result<()> {
        let path_bytes = row.path.as_bytes();
        let path_len: u32 = path_bytes.len().try_into().wrap_err_with(|| {
            format!(
                "Path too long to encode in index row ({} bytes)",
                path_bytes.len()
            )
        })?;

        let normalized_path = row.path.to_lowercase();
        let normalized_path_bytes = normalized_path.as_bytes();
        let normalized_path_len: u32 =
            normalized_path_bytes.len().try_into().wrap_err_with(|| {
                format!(
                    "Normalized path too long to encode in index row ({} bytes)",
                    normalized_path_bytes.len()
                )
            })?;

        self.bytes.extend_from_slice(&path_len.to_le_bytes());
        self.bytes
            .extend_from_slice(&normalized_path_len.to_le_bytes());
        self.bytes
            .extend_from_slice(&[u8::from(row.has_deleted_entries)]);
        self.bytes.extend_from_slice(path_bytes);
        self.bytes.extend_from_slice(normalized_path_bytes);

        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if any row path or normalized path is too long to encode.
    pub fn extend_rows<'a>(
        &mut self,
        rows: impl IntoIterator<Item = &'a SearchIndexPathRow>,
    ) -> eyre::Result<()> {
        for row in rows {
            self.push_row(row)?;
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if any row path or normalized path is too long to encode.
    pub fn from_rows(header: SearchIndexHeader, rows: &[SearchIndexPathRow]) -> eyre::Result<Self> {
        if header.version != SEARCH_INDEX_VERSION {
            bail!(
                "Cannot build search index bytes with unsupported version {} (expected {})",
                header.version,
                SEARCH_INDEX_VERSION
            );
        }

        let mut bytes = Self::new(header);
        bytes.extend_rows(rows.iter())?;
        Ok(bytes)
    }

    #[must_use]
    pub fn header(&self) -> SearchIndexHeader {
        self.header
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }

    /// Write the search index to `output_path` atomically via a temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error if the output file cannot be created or written,
    /// or if the temporary file cannot be renamed into place.
    pub fn write_to_path(self, output_path: impl AsRef<Path>) -> eyre::Result<()> {
        let output_path = output_path.as_ref();
        let temp_path = output_path.with_extension("mft_search_index.tmp");

        let file = std::fs::File::create(&temp_path).wrap_err_with(|| {
            format!(
                "Failed creating temporary search index file {}",
                temp_path.display()
            )
        })?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(&self.bytes)?;
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
}
