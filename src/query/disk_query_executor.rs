use crate::machine::config::published_drive_paths;
use crate::query::QueryFilterBehavior;
use crate::query::QueryFilterRules;
use crate::query::QueryPlan;
use crate::query::QueryRowFilter;
use crate::query::QueryResultRow;
use crate::query::visit_drive_search_index_rows;
use eyre::bail;
use std::ops::ControlFlow;
use std::path::PathBuf;
use tracing::debug;
use tracing::info_span;

/// A one-shot local published-index visitor helper.
///
/// New backend-agnostic callers should generally prefer `QueryRuntime`, and
/// repeated in-process callers should prefer `QuerySession`. `DiskQueryExecutor`
/// remains as the specialized direct local query helper that still exposes
/// explicit `QueryFilterBehavior` control.
#[derive(Debug)]
pub struct DiskQueryExecutor {
    pub sync_dir: PathBuf,
    pub mft_files: Vec<(char, PathBuf)>,
    pub query_plan: QueryPlan,
    pub filter_behavior: QueryFilterBehavior,
}

impl DiskQueryExecutor {
    /// # Errors
    ///
    /// Returns an error if drive letters cannot be resolved, the sync directory
    /// cannot be loaded, or a selected drive has no published MFT snapshot.
    pub fn new(query_plan: QueryPlan) -> eyre::Result<Self> {
        let drive_letters = query_plan.drive_letter_pattern.into_drive_letters()?;
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
            query_plan,
            filter_behavior: QueryFilterBehavior::AutoDiscover,
        })
    }

    /// # Errors
    ///
    /// Returns an error if query parsing, scope resolution, or filter-rule discovery fails.
    pub fn visit_rows(
        self,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
    ) -> eyre::Result<()> {
        let _span = info_span!("query_execute").entered();
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
                        self.query_plan.profile.as_deref(),
                    )?)
                }
                QueryFilterBehavior::Disabled => None,
                QueryFilterBehavior::Custom(rules) => Some(rules),
            }
        };
        let filter = QueryRowFilter::new(&self.query_plan, filter_rules)?;
        let include_deleted = self.query_plan.include_deleted;
        let only_deleted = self.query_plan.only_deleted;
        let mut should_stop = false;

        for (drive_letter, _) in self.mft_files {
            if should_stop {
                break;
            }
            let loaded_rows = visit_drive_search_index_rows(
                drive_letter,
                &self.sync_dir,
                &self.query_plan,
                include_deleted,
                only_deleted,
                |row| {
                    let Some(row) = filter.classify_and_match(row) else {
                        return Ok(ControlFlow::Continue(()));
                    };
                    match visit(row)? {
                        ControlFlow::Continue(()) => Ok(ControlFlow::Continue(())),
                        ControlFlow::Break(()) => {
                            should_stop = true;
                            Ok(ControlFlow::Break(()))
                        }
                    }
                },
            )?;
            debug!(
                drive = %drive_letter,
                loaded_rows,
                "Disk query drive completed"
            );
        }

        Ok(())
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
    use std::ops::ControlFlow;

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
                    path: cargo_path.display().to_string().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: rules_path.display().to_string().into(),
                    has_deleted_entries: false,
                },
            ],
        )?;

        let mut request = QueryPlan::new("Cargo.toml");
        request.drive_letter_pattern = DriveLetterPattern(String::from("C"));

        let auto_discover = DiskQueryExecutor {
            sync_dir: temp_dir.path().to_path_buf(),
            mft_files: vec![('C', paths.mft_path.clone())],
            query_plan: request.clone(),
            filter_behavior: QueryFilterBehavior::AutoDiscover,
        };
        let disabled = DiskQueryExecutor {
            sync_dir: temp_dir.path().to_path_buf(),
            mft_files: vec![('C', paths.mft_path)],
            query_plan: request.clone(),
            filter_behavior: QueryFilterBehavior::Disabled,
        };
        let mut auto_rows = Vec::new();
        auto_discover.visit_rows(|row| {
            auto_rows.push(row);
            Ok(ControlFlow::Continue(()))
        })?;
        let mut disabled_rows = Vec::new();
        disabled.visit_rows(|row| {
            disabled_rows.push(row);
            Ok(ControlFlow::Continue(()))
        })?;

        assert!(auto_rows.is_empty());
        assert_eq!(disabled_rows.len(), 1);
        assert_eq!(disabled_rows[0].path.as_path(), cargo_path.as_path());
        Ok(())
    }
}
