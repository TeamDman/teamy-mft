use crate::machine::config::published_drive_paths;
use crate::query::ControlFlow;
use crate::query::QueryFilterBehavior;
use crate::query::QueryFilterRules;
use crate::query::QueryPlan;
use crate::query::QueryRowFilter;
use crate::query::QueryRowSink;
use crate::query::QueryRowStream;
use crate::query::visit_drive_search_index_rows;
use eyre::bail;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;
use tracing::info_span;

/// A one-shot local published-index streaming helper.
///
/// New backend-agnostic callers should generally prefer `QueryRuntime`, and
/// repeated in-process callers should prefer `QuerySession`. `DiskQueryExecutor`
/// remains as the specialized direct local streaming helper that still exposes
/// explicit `QueryFilterBehavior` control.
#[derive(Debug)]
pub struct DiskQueryExecutor {
    pub sync_dir: PathBuf,
    pub mft_files: Vec<(char, PathBuf)>,
    pub request: QueryPlan,
    pub filter_behavior: QueryFilterBehavior,
}

impl DiskQueryExecutor {
    /// # Errors
    ///
    /// Returns an error if drive letters cannot be resolved, the sync directory
    /// cannot be loaded, or a selected drive has no published MFT snapshot.
    pub fn new(request: QueryPlan) -> eyre::Result<Self> {
        let drive_letters = request.drive_letter_pattern.into_drive_letters()?;
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let mft_files = {
            let _span = info_span!("discover_mft_files").entered();
            drive_letters
                .iter()
                .copied()
                .map(|drive_letter| {
                    let paths = published_drive_paths(&sync_dir, drive_letter);
                    (drive_letter, paths.mft_path)
                })
                .map(|(drive_letter, drive_mft_file_path)| {
                    if drive_mft_file_path.is_file() {
                        Ok((drive_letter, drive_mft_file_path))
                    } else {
                        bail!(
                            "MFT file for drive {} not found at expected path: {}",
                            drive_letter,
                            drive_mft_file_path.display()
                        )
                    }
                })
                .collect::<eyre::Result<Vec<_>>>()?
        };

        Ok(Self {
            sync_dir,
            mft_files,
            request,
            filter_behavior: QueryFilterBehavior::AutoDiscover,
        })
    }

    /// # Errors
    ///
    /// Returns an error if query parsing, scope resolution, or filter-rule discovery fails.
    pub fn stream(self) -> eyre::Result<QueryRowStream> {
        let _span = info_span!("query_execute").entered();
        let query_plan = Arc::new(self.request.clone());
        let drive_letters = self
            .mft_files
            .iter()
            .map(|(drive_letter, _)| *drive_letter)
            .collect::<Vec<_>>();
        let filter_rules = {
            let _span = info_span!("query_prepare_filters").entered();
            match self.filter_behavior {
                QueryFilterBehavior::AutoDiscover => {
                    Some(QueryFilterRules::discover_for_drive_letters(
                        &drive_letters,
                        &self.sync_dir,
                        self.request.profile.as_deref(),
                    )?)
                }
                QueryFilterBehavior::Disabled => None,
                QueryFilterBehavior::Custom(rules) => Some(rules),
            }
        };
        let filter = Arc::new(QueryRowFilter::new(&self.request, filter_rules)?);
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let sink = QueryRowSink::new(tx);
        let drive_count = self.mft_files.len();
        let sync_dir = Arc::new(self.sync_dir);
        let include_deleted = self.request.include_deleted;
        let only_deleted = self.request.only_deleted;

        std::thread::Builder::new()
            .name("teamy-mft-query-disk-producers".to_owned())
            .spawn(move || {
                let _span = info_span!("query_disk_producers").entered();
                let mut handles = Vec::with_capacity(drive_count);
                for (drive_letter, _) in self.mft_files {
                    let sink = sink.clone();
                    let query_plan = Arc::clone(&query_plan);
                    let filter = Arc::clone(&filter);
                    let sync_dir = Arc::clone(&sync_dir);
                    let thread_name = format!("teamy-mft-query-drive-{drive_letter}");
                    handles.push(
                        std::thread::Builder::new()
                            .name(thread_name)
                            .spawn(move || {
                                let _span = info_span!("query_drive_task").entered();
                                let result = visit_drive_search_index_rows(
                                    drive_letter,
                                    &sync_dir,
                                    &query_plan,
                                    include_deleted,
                                    only_deleted,
                                    |row| {
                                        let Some(row) = filter.classify_and_match(row) else {
                                            return Ok(ControlFlow::Continue);
                                        };
                                        Ok(if sink.blocking_send(row).is_ok() {
                                            ControlFlow::Continue
                                        } else {
                                            ControlFlow::Break
                                        })
                                    },
                                );
                                match result {
                                    Ok(loaded_rows) => {
                                        debug!(
                                            drive = %drive_letter,
                                            loaded_rows,
                                            "Disk query drive completed"
                                        );
                                    }
                                    Err(error) => {
                                        let _ = sink.blocking_send_error(error);
                                    }
                                }
                            }),
                    );
                }
                for handle in handles {
                    match handle {
                        Ok(handle) => {
                            let _ = handle.join();
                        }
                        Err(error) => {
                            let _ = sink.blocking_send_error(error.into());
                        }
                    }
                }
            })?;

        Ok(QueryRowStream::Local(rx))
    }
}

#[cfg(test)]
mod tests {
    use super::DiskQueryExecutor;
    use crate::machine::config::published_drive_paths;
    use crate::query::QueryFilterBehavior;
    use crate::query::QueryPlan;
    use crate::query::RULES_FILE_EXTENSION;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use crate::windows_utils::storage::DriveLetterPattern;

    fn write_index(
        path: &std::path::Path,
        drive: char,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<()> {
        std::fs::create_dir_all(
            path.parent()
                .expect("index path should have a parent directory"),
        )?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, 123, rows.len() as u64),
            rows,
        )?
        .write_to_path(path)?;
        Ok(())
    }

    #[test]
    fn filter_behavior_disabled_bypasses_auto_discovered_rules() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let paths = published_drive_paths(temp_dir.path(), 'C');
        std::fs::create_dir_all(
            paths
                .mft_path
                .parent()
                .expect("mft path should have a parent directory"),
        )?;
        std::fs::write(&paths.mft_path, b"placeholder mft snapshot")?;

        let cargo_path = temp_dir.path().join("Cargo.toml");
        let rules_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        std::fs::write(&rules_path, format!("EXCLUDE {}\n", cargo_path.display()))?;
        write_index(
            &paths.base_index_path,
            'C',
            &[
                SearchIndexPathRow {
                    path: cargo_path.display().to_string(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: rules_path.display().to_string(),
                    has_deleted_entries: false,
                },
            ],
        )?;

        let mut request = QueryPlan::new("Cargo.toml");
        request.drive_letter_pattern = DriveLetterPattern(String::from("C"));

        let auto_discover = DiskQueryExecutor {
            sync_dir: temp_dir.path().to_path_buf(),
            mft_files: vec![('C', paths.mft_path.clone())],
            request: request.clone(),
            filter_behavior: QueryFilterBehavior::AutoDiscover,
        };
        let disabled = DiskQueryExecutor {
            sync_dir: temp_dir.path().to_path_buf(),
            mft_files: vec![('C', paths.mft_path)],
            request: request.clone(),
            filter_behavior: QueryFilterBehavior::Disabled,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let auto_rows = runtime.block_on(
            auto_discover
                .stream()?
                .collect_filtered_limit(request.limit),
        )?;
        let disabled_rows =
            runtime.block_on(disabled.stream()?.collect_filtered_limit(request.limit))?;

        assert!(auto_rows.is_empty());
        assert_eq!(disabled_rows.len(), 1);
        assert_eq!(disabled_rows[0].path.as_path(), cargo_path.as_path());
        Ok(())
    }
}
