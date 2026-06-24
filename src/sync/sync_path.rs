use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::PublishedDrivePaths;
use crate::machine::config::current_unix_ms;
use crate::machine::config::load_checkpoint;
use crate::machine::config::published_drive_paths;
use crate::machine::config::save_checkpoint;
use crate::query::QueryPlan;
use crate::query::QueryScope;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use crate::search_index::search_index_bytes::SearchIndexBytesMut;
use eyre::Context;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::ops::ControlFlow;
use std::path::Path;
use std::path::PathBuf;

/// # Errors
///
/// Returns an error if the input path is empty, cannot be resolved to an absolute drive path,
/// the published base index is missing, or the overlay/checkpoint cannot be updated.
pub fn sync_path_into_published_overlay(sync_dir: &Path, path: &str) -> eyre::Result<char> {
    let target_path = resolve_sync_target_path(path)?;
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
    let mut rows_by_path = if published_paths.overlay_index_path.is_file() {
        load_rows_from_index(&published_paths.overlay_index_path)?
    } else {
        Vec::new()
    }
    .into_iter()
    .map(|row| (row.path.clone(), row))
    .collect::<BTreeMap<_, _>>();
    rows_by_path.insert(
        rendered_path.clone().into(),
        SearchIndexPathRow {
            path: rendered_path.into(),
            has_deleted_entries: !target_path.exists(),
        },
    );
    let overlay_rows = rows_by_path.into_values().collect::<Vec<_>>();

    write_overlay_rows_and_checkpoint(&published_paths, &overlay_rows)?;

    Ok(drive_letter)
}

/// # Errors
///
/// Returns an error if the input path is empty, does not resolve to a directory subtree on an
/// absolute Windows drive, the published base index is missing, or the overlay/checkpoint cannot
/// be updated.
pub fn sync_path_recursively_into_published_overlay(
    sync_dir: &Path,
    path: &str,
) -> eyre::Result<char> {
    let target_path = resolve_sync_target_path(path)?;
    if target_path.exists() && !target_path.is_dir() {
        eyre::bail!(
            "Recursive sync requires a directory path, but {} is not a directory",
            target_path.display()
        );
    }

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

    let scope = QueryScope {
        root: target_path.clone(),
        include_descendants: true,
    };
    let base_rows = load_rows_from_index_in_scope(&published_paths.base_index_path, &scope)?;
    let current_rows = collect_recursive_rows(&target_path)?;

    let mut rows_by_path = if published_paths.overlay_index_path.is_file() {
        load_rows_from_index(&published_paths.overlay_index_path)?
    } else {
        Vec::new()
    }
    .into_iter()
    .filter(|row| !scope.matches_path(row.path.as_path()))
    .map(|row| (row.path.clone(), row))
    .collect::<BTreeMap<_, _>>();

    let base_deleted_by_path = base_rows
        .into_iter()
        .map(|row| (row.path.clone(), row.has_deleted_entries))
        .collect::<BTreeMap<_, _>>();
    let current_deleted_by_path = current_rows
        .into_iter()
        .map(|row| (row.path.clone(), row.has_deleted_entries))
        .collect::<BTreeMap<_, _>>();

    for (path, &has_deleted_entries) in &current_deleted_by_path {
        match base_deleted_by_path.get(path) {
            Some(base_deleted) if *base_deleted == has_deleted_entries => {}
            _ => {
                rows_by_path.insert(
                    path.clone(),
                    SearchIndexPathRow {
                        path: path.clone(),
                        has_deleted_entries,
                    },
                );
            }
        }
    }
    for path in base_deleted_by_path.keys() {
        if current_deleted_by_path.contains_key(path) {
            continue;
        }
        rows_by_path.insert(
            path.clone(),
            SearchIndexPathRow {
                path: path.clone(),
                has_deleted_entries: true,
            },
        );
    }

    let overlay_rows = rows_by_path.into_values().collect::<Vec<_>>();
    write_overlay_rows_and_checkpoint(&published_paths, &overlay_rows)?;

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

fn load_rows_from_index_in_scope(
    path: &Path,
    scope: &QueryScope,
) -> eyre::Result<Vec<SearchIndexPathRow>> {
    let mapped = MappedSearchIndex::open(path)
        .wrap_err_with(|| format!("Failed loading search index rows from {}", path.display()))?;
    let parsed = SearchIndexBytes::new(mapped.bytes())
        .parse_trusted_for_query()
        .wrap_err_with(|| format!("Failed preparing search index rows from {}", path.display()))?;
    let query_plan = QueryPlan {
        include_deleted: true,
        ..QueryPlan::default()
    };
    let mut rows = Vec::new();
    let _ = crate::query::visit_parsed_search_index_rows(
        &parsed,
        &query_plan,
        std::slice::from_ref(scope),
        true,
        false,
        |row| {
            rows.push(SearchIndexPathRow {
                path: row.path,
                has_deleted_entries: row.has_deleted_entries,
            });
            Ok(ControlFlow::Continue(()))
        },
    )?;
    Ok(rows)
}

fn collect_recursive_rows(target_path: &Path) -> eyre::Result<Vec<SearchIndexPathRow>> {
    if !target_path.exists() {
        return Ok(Vec::new());
    }

    let mut rows = vec![SearchIndexPathRow {
        path: target_path.display().to_string().into(),
        has_deleted_entries: false,
    }];
    let mut directories = vec![target_path.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)
            .wrap_err_with(|| format!("Failed reading directory {}", directory.display()))?
        {
            let entry = entry.wrap_err_with(|| {
                format!("Failed reading directory entry in {}", directory.display())
            })?;
            let entry_path = entry.path();
            let file_type = entry.file_type().wrap_err_with(|| {
                format!("Failed reading file type for {}", entry_path.display())
            })?;
            rows.push(SearchIndexPathRow {
                path: entry_path.display().to_string().into(),
                has_deleted_entries: false,
            });
            if file_type.is_dir() {
                directories.push(entry_path);
            }
        }
    }

    Ok(rows)
}

fn write_overlay_rows_and_checkpoint(
    published_paths: &PublishedDrivePaths,
    overlay_rows: &[SearchIndexPathRow],
) -> eyre::Result<()> {
    SearchIndexBytesMut::from_rows(
        SearchIndexHeader::new(published_paths.drive_letter, 0, overlay_rows.len() as u64),
        overlay_rows,
    )?
    .write_to_path(&published_paths.overlay_index_path)?;

    let mut checkpoint = load_checkpoint(&published_paths.checkpoint_path)?.unwrap_or_else(|| {
        PublishedCheckpoint::empty(published_paths.drive_letter, SEARCH_INDEX_VERSION)
    });
    checkpoint.published_at_unix_ms = current_unix_ms();
    checkpoint.overlay_row_count = overlay_rows.len() as u64;
    checkpoint.base_index_version = SEARCH_INDEX_VERSION;
    save_checkpoint(&published_paths.checkpoint_path, &checkpoint)?;
    Ok(())
}

fn resolve_sync_target_path(path: &str) -> eyre::Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        eyre::bail!("sync path must not be empty");
    }

    let target_path = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        std::env::current_dir()?.join(trimmed)
    };
    if target_path.exists() {
        return Ok(dunce::canonicalize(&target_path)?);
    }

    let mut suffix = Vec::<OsString>::new();
    let mut existing_ancestor = target_path.as_path();
    while !existing_ancestor.exists() {
        let Some(file_name) = existing_ancestor.file_name() else {
            return Ok(target_path);
        };
        suffix.push(file_name.to_owned());
        let Some(parent) = existing_ancestor.parent() else {
            return Ok(target_path);
        };
        existing_ancestor = parent;
    }

    let mut resolved = dunce::canonicalize(existing_ancestor)?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
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
    use super::drive_letter_for_absolute_windows_path;
    use super::sync_path_into_published_overlay;
    use super::sync_path_recursively_into_published_overlay;
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
                path: String::from(r"C:\existing.txt").into(),
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
                .any(|row| row.path.as_str() == r"C:\existing.txt")
        );
        assert!(
            overlay_rows
                .iter()
                .any(|row| row.path.as_str() == r"C:\repo\filters.rules")
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

    #[cfg(windows)]
    #[test]
    fn recursive_sync_replaces_subtree_overlay_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let repo_dir = temp_dir.path().join("repo");
        let nested_dir = repo_dir.join("nested");
        std::fs::create_dir_all(&nested_dir)?;
        std::fs::write(repo_dir.join("new.txt"), [])?;
        std::fs::write(nested_dir.join("kept.txt"), [])?;

        let repo_dir = dunce::canonicalize(&repo_dir)?;
        let nested_dir = repo_dir.join("nested");
        let kept_path = nested_dir.join("kept.txt");
        let deleted_path = repo_dir.join("deleted.txt");
        let new_path = repo_dir.join("new.txt");
        let stale_overlay_path = repo_dir.join("stale.txt");
        let outside_overlay_path = temp_dir.path().join("outside.txt");

        let drive = drive_letter_for_absolute_windows_path(&repo_dir.display().to_string())?;
        let published_paths = published_drive_paths(temp_dir.path(), drive);
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, 123, 4),
            &[
                SearchIndexPathRow {
                    path: repo_dir.display().to_string().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: nested_dir.display().to_string().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: kept_path.display().to_string().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: deleted_path.display().to_string().into(),
                    has_deleted_entries: false,
                },
            ],
        )?
        .write_to_path(&published_paths.base_index_path)?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, 0, 2),
            &[
                SearchIndexPathRow {
                    path: stale_overlay_path.display().to_string().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: outside_overlay_path.display().to_string().into(),
                    has_deleted_entries: false,
                },
            ],
        )?
        .write_to_path(&published_paths.overlay_index_path)?;
        crate::machine::config::save_checkpoint(
            &published_paths.checkpoint_path,
            &PublishedCheckpoint::empty(drive, SEARCH_INDEX_VERSION),
        )?;

        let synced_drive = sync_path_recursively_into_published_overlay(
            temp_dir.path(),
            &repo_dir.display().to_string(),
        )?;

        assert_eq!(synced_drive, drive);
        let overlay_rows = super::load_rows_from_index(&published_paths.overlay_index_path)?;
        assert!(
            overlay_rows
                .iter()
                .any(|row| row.path.as_str() == outside_overlay_path.display().to_string())
        );
        assert!(
            overlay_rows
                .iter()
                .any(|row| row.path.as_str() == new_path.display().to_string()
                    && !row.has_deleted_entries)
        );
        assert!(overlay_rows.iter().any(|row| row.path.as_str()
            == deleted_path.display().to_string()
            && row.has_deleted_entries));
        assert!(
            overlay_rows
                .iter()
                .all(|row| row.path.as_str() != kept_path.display().to_string())
        );
        assert!(
            overlay_rows
                .iter()
                .all(|row| row.path.as_str() != stale_overlay_path.display().to_string())
        );
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn recursive_sync_requires_directory_path() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let file_path = temp_dir.path().join("filters.teamy_mft_rules");
        std::fs::write(&file_path, "content")?;
        let file_path = dunce::canonicalize(&file_path)?;

        let error = sync_path_recursively_into_published_overlay(
            temp_dir.path(),
            &file_path.display().to_string(),
        )
        .expect_err("existing file path should be rejected for recursive sync");

        assert!(error.to_string().contains("is not a directory"));
        Ok(())
    }
}
