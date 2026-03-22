use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use crate::search_index::search_index_bytes::SearchIndexRowIter;
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

        let search_index_bytes = SearchIndexBytes::new(&mmap);
        let header = search_index_bytes.header().wrap_err_with(|| {
            format!("Failed parsing search index header from {}", path.display())
        })?;

        if header.version != SEARCH_INDEX_VERSION {
            let drive_letter = char::from(header.drive_letter);
            bail!(
                "Unsupported search index version {} in {}. Run `teamy-mft sync index --drive-pattern {}` to rebuild the stale index for drive {}.",
                header.version,
                path.display(),
                drive_letter,
                drive_letter
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
        SearchIndexBytes::new(&self.mmap).row_views()
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
    use crate::search_index::format::SEARCH_INDEX_MAGIC;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use std::fs;

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

        SearchIndexBytesMut::from_rows(SearchIndexHeader::new('C', 123, rows.len() as u64), &rows)?
            .write_to_path(&index_path)?;

        let mapped = MappedSearchIndex::open(&index_path)?;
        let views = mapped.row_views()?.collect::<eyre::Result<Vec<_>>>()?;

        assert_eq!(views.len(), 2);
        assert_eq!(views[0].path(), "C:\\alpha.txt");
        assert_eq!(
            views[0]
                .segment_views()
                .map(|segment| segment.normalized)
                .collect::<Vec<_>>(),
            vec!["alpha.txt", "c:"]
        );
        assert!(!views[0].has_deleted_entries);
        assert_eq!(views[1].path(), "C:\\beta.LOG");
        assert_eq!(
            views[1]
                .segment_views()
                .map(|segment| segment.normalized)
                .collect::<Vec<_>>(),
            vec!["beta.log", "c:"]
        );
        assert!(views[1].has_deleted_entries);

        let bytes = mapped.bytes();
        let bytes_start = bytes.as_ptr() as usize;
        let bytes_end = bytes_start + bytes.len();
        let first_segment = views[0]
            .segment_views()
            .next()
            .expect("row should contain at least one path segment");
        let first_ptr = first_segment.display.as_ptr() as usize;
        assert!((bytes_start..bytes_end).contains(&first_ptr));

        Ok(())
    }

    #[test]
    fn opening_legacy_v1_indexes_prompts_a_sync_rebuild() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let index_path = temp_dir.path().join("C.mft_search_index");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(SEARCH_INDEX_MAGIC);
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.push(b'C');
        bytes.extend_from_slice(&123u64.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());

        let path = b"C:\\legacy.txt";
        bytes.extend_from_slice(&(path.len() as u32).to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(path);

        assert!(!bytes.is_empty());
        fs::write(&index_path, bytes)?;

        let error =
            MappedSearchIndex::open(&index_path).expect_err("legacy index should be rejected");
        let message = error.to_string();

        assert!(message.contains("Unsupported search index version 1"));
        assert!(message.contains("teamy-mft sync index --drive-pattern C"));

        Ok(())
    }
}
