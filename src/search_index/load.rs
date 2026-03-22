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

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexPathRowView<'a> {
    pub path: &'a str,
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

impl<'a> Iterator for SearchIndexRowIter<'a> {
    type Item = eyre::Result<SearchIndexPathRowView<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_rows == 0 {
            return None;
        }

        let row_index = self.row_index;
        self.row_index += 1;
        self.remaining_rows -= 1;

        if self.bytes.len() < self.cursor + 4 + 1 {
            return Some(Err(eyre::eyre!(
                "Corrupt search index: truncated row header at row {}",
                row_index
            )));
        }

        let len_start = self.cursor;
        self.cursor += 4;
        let path_len = u32::from_le_bytes([
            self.bytes[len_start],
            self.bytes[len_start + 1],
            self.bytes[len_start + 2],
            self.bytes[len_start + 3],
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
        self.cursor = path_end;

        Some(Ok(SearchIndexPathRowView {
            path,
            has_deleted_entries,
        }))
    }
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

    /// Iterate over path rows from the mapped search index without allocating.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored node count does not fit in `usize`.
    pub fn row_views(&self) -> eyre::Result<SearchIndexRowIter<'_>> {
        let node_count = usize::try_from(self.header.node_count).wrap_err_with(|| {
            format!(
                "Search index node count {} does not fit into usize",
                self.header.node_count
            )
        })?;

        Ok(SearchIndexRowIter {
            bytes: &self.mmap,
            cursor: SEARCH_INDEX_HEADER_LEN,
            remaining_rows: node_count,
            row_index: 0,
        })
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

        for row in self.row_views()? {
            rows.push(row?.to_owned());
        }

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::MappedSearchIndex;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;

    #[test]
    fn row_views_reads_paths_without_materializing_all_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let index_path = temp_dir.path().join("C.mft_search_index");
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\alpha.txt"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\beta.LOG"),
                has_deleted_entries: true,
            },
        ];

        SearchIndexHeader::new('C', 123, rows.len() as u64).write_to_path(&index_path, &rows)?;

        let mapped = MappedSearchIndex::open(&index_path)?;
        let views = mapped.row_views()?.collect::<eyre::Result<Vec<_>>>()?;

        assert_eq!(views.len(), 2);
        assert_eq!(views[0].path, "C:\\alpha.txt");
        assert!(!views[0].has_deleted_entries);
        assert_eq!(views[1].path, "C:\\beta.LOG");
        assert!(views[1].has_deleted_entries);

        let bytes = mapped.bytes();
        let bytes_start = bytes.as_ptr() as usize;
        let bytes_end = bytes_start + bytes.len();
        let first_ptr = views[0].path.as_ptr() as usize;
        assert!((bytes_start..bytes_end).contains(&first_ptr));

        Ok(())
    }
}
