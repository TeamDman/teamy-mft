use crate::search_index::format::SEARCH_INDEX_HEADER_LEN;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use eyre::Context;
use eyre::bail;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

#[derive(Debug)]
pub struct MappedSearchIndex {
    mmap: Mmap,
    pub header: SearchIndexHeader,
}

impl MappedSearchIndex {
    /// Open and validate a memory-mapped search index file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or mapped, if the header
    /// cannot be parsed, or if the index version is unsupported.
    pub fn open(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .wrap_err_with(|| format!("Failed opening search index file {}", path.display()))?;

        // SAFETY: `file` remains alive for the duration of this call, and the
        // resulting `Mmap` owns its mapping independently after creation.
        let mmap = unsafe { Mmap::map(&file) }.wrap_err_with(|| {
            format!("Failed memory-mapping search index file {}", path.display())
        })?;

        let header = SearchIndexHeader::parse(&mmap).wrap_err_with(|| {
            format!("Failed parsing search index header from {}", path.display())
        })?;

        if header.version != SEARCH_INDEX_VERSION {
            bail!(
                "Unsupported search index version {} in {} (expected {})",
                header.version,
                path.display(),
                SEARCH_INDEX_VERSION
            );
        }

        Ok(Self { mmap, header })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.mmap
    }

    /// Parse all path rows from the mapped search index.
    ///
    /// # Errors
    ///
    /// Returns an error if the mapped bytes are truncated or malformed,
    /// or if a row path is not valid UTF-8.
    pub fn rows(&self) -> eyre::Result<Vec<SearchIndexPathRow>> {
        let node_count = usize::try_from(self.header.node_count).wrap_err_with(|| {
            format!(
                "Search index node count {} does not fit into usize",
                self.header.node_count
            )
        })?;
        let mut rows = Vec::with_capacity(node_count);
        let mut cursor = SEARCH_INDEX_HEADER_LEN;

        for row_index in 0..self.header.node_count {
            if self.mmap.len() < cursor + 4 + 1 {
                bail!(
                    "Corrupt search index: truncated row header at row {}",
                    row_index
                );
            }

            let len_start = cursor;
            cursor += 4;
            let path_len = u32::from_le_bytes([
                self.mmap[len_start],
                self.mmap[len_start + 1],
                self.mmap[len_start + 2],
                self.mmap[len_start + 3],
            ]) as usize;

            let has_deleted_entries = self.mmap[cursor] != 0;
            cursor += 1;

            let path_end = cursor + path_len;
            if self.mmap.len() < path_end {
                bail!(
                    "Corrupt search index: truncated path payload at row {}",
                    row_index
                );
            }

            let path = std::str::from_utf8(&self.mmap[cursor..path_end])
                .wrap_err_with(|| format!("Invalid UTF-8 path payload at row {row_index}"))?
                .to_owned();
            cursor = path_end;

            rows.push(SearchIndexPathRow {
                path,
                has_deleted_entries,
            });
        }

        Ok(rows)
    }
}
