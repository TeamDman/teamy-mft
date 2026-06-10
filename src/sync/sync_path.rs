use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::current_unix_ms;
use crate::machine::config::load_checkpoint;
use crate::machine::config::published_drive_paths;
use crate::machine::config::save_checkpoint;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use crate::search_index::search_index_bytes::SearchIndexBytesMut;
use eyre::Context;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

/// # Errors
///
/// Returns an error if the input path is empty, cannot be resolved to an absolute drive path,
/// the published base index is missing, or the overlay/checkpoint cannot be updated.
pub fn sync_path_into_published_overlay(sync_dir: &Path, path: &str) -> eyre::Result<char> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        eyre::bail!("sync path must not be empty");
    }

    let target_path = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        std::env::current_dir()?.join(trimmed)
    };
    let target_path = if target_path.exists() {
        dunce::canonicalize(&target_path)?
    } else {
        target_path
    };

    let rendered_path = target_path.display().to_string();
    let drive_letter = drive_letter_for_absolute_windows_path(&rendered_path)?;
    let published_paths = published_drive_paths(sync_dir, drive_letter);
    if !published_paths.base_index_path.is_file() {
        eyre::bail!(
            "Cannot sync path {} because {} is missing. Run `teamy-mft sync --drive {}` first.",
            rendered_path,
            published_paths.base_index_path.display(),
            drive_letter
        );
    }

    let mut overlay_rows = if published_paths.overlay_index_path.is_file() {
        load_rows_from_index(&published_paths.overlay_index_path)?
    } else {
        Vec::new()
    };
    let mut rows_by_path = overlay_rows
        .drain(..)
        .map(|row| (row.path.clone(), row))
        .collect::<BTreeMap<_, _>>();
    rows_by_path.insert(
        rendered_path.clone(),
        SearchIndexPathRow {
            path: rendered_path,
            has_deleted_entries: !target_path.exists(),
        },
    );
    let overlay_rows = rows_by_path.into_values().collect::<Vec<_>>();

    SearchIndexBytesMut::from_rows(
        SearchIndexHeader::new(drive_letter, 0, overlay_rows.len() as u64),
        &overlay_rows,
    )?
    .write_to_path(&published_paths.overlay_index_path)?;

    let mut checkpoint = load_checkpoint(&published_paths.checkpoint_path)?
        .unwrap_or_else(|| PublishedCheckpoint::empty(drive_letter, SEARCH_INDEX_VERSION));
    checkpoint.published_at_unix_ms = current_unix_ms();
    checkpoint.overlay_row_count = overlay_rows.len() as u64;
    checkpoint.base_index_version = SEARCH_INDEX_VERSION;
    save_checkpoint(&published_paths.checkpoint_path, &checkpoint)?;

    Ok(drive_letter)
}

fn load_rows_from_index(path: &Path) -> eyre::Result<Vec<SearchIndexPathRow>> {
    let bytes = std::fs::read(path)
        .wrap_err_with(|| format!("Failed reading search index rows from {}", path.display()))?;
    SearchIndexBytes::new(&bytes)
        .row_views()?
        .map(|row| {
            row.map(
                super::super::search_index::search_index_bytes::SearchIndexPathRowView::to_owned,
            )
        })
        .collect()
}

fn drive_letter_for_absolute_windows_path(path: &str) -> eyre::Result<char> {
    let bytes = path.as_bytes();
    if bytes.len() < 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        eyre::bail!(
            "Path {} must resolve to an absolute Windows drive path like C:\\repo\\file.rules",
            path
        );
    }
    Ok(char::from(bytes[0].to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::sync_path_into_published_overlay;
    use crate::machine::config::PublishedCheckpoint;
    use crate::machine::config::load_checkpoint;
    use crate::machine::config::published_drive_paths;
    use crate::search_index::format::SEARCH_INDEX_VERSION;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;

    #[test]
    fn sync_path_into_overlay_upserts_one_row_and_checkpoint() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let paths = published_drive_paths(temp_dir.path(), 'C');
        SearchIndexBytesMut::from_rows(SearchIndexHeader::new('C', 123, 0), &[])?
            .write_to_path(&paths.base_index_path)?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 0, 1),
            &[SearchIndexPathRow {
                path: String::from(r"C:\existing.txt"),
                has_deleted_entries: false,
            }],
        )?
        .write_to_path(&paths.overlay_index_path)?;
        crate::machine::config::save_checkpoint(
            &paths.checkpoint_path,
            &PublishedCheckpoint::empty('C', SEARCH_INDEX_VERSION),
        )?;

        let drive = sync_path_into_published_overlay(temp_dir.path(), r"C:\repo\filters.rules")?;

        assert_eq!(drive, 'C');
        let overlay_bytes = std::fs::read(&paths.overlay_index_path)?;
        let overlay_rows = SearchIndexBytes::new(&overlay_bytes)
            .row_views()?
            .map(|row| row.map(|view| view.to_owned()))
            .collect::<eyre::Result<Vec<_>>>()?;
        assert!(
            overlay_rows
                .iter()
                .any(|row| row.path == r"C:\existing.txt")
        );
        assert!(
            overlay_rows
                .iter()
                .any(|row| row.path == r"C:\repo\filters.rules")
        );
        let checkpoint = load_checkpoint(&paths.checkpoint_path)?.expect("checkpoint should exist");
        assert_eq!(checkpoint.overlay_row_count, 2);
        Ok(())
    }

    #[test]
    fn sync_path_requires_existing_base_index() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let error = sync_path_into_published_overlay(temp_dir.path(), r"C:\repo\filters.rules")
            .expect_err("base index should be required");
        assert!(
            error
                .to_string()
                .contains("Run `teamy-mft sync --drive C` first.")
        );
    }
}
