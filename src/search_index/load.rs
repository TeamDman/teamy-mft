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
    pub fn open(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .wrap_err_with(|| format!("Failed opening search index file {}", path.display()))?;

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

    pub fn rows(&self) -> eyre::Result<Vec<SearchIndexPathRow>> {
        let mut rows = Vec::with_capacity(self.header.node_count as usize);
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
                .wrap_err_with(|| format!("Invalid UTF-8 path payload at row {}", row_index))?
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
