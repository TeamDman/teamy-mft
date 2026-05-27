use crate::query::QueryFilter;
use crate::query::QueryIgnoreBehavior;
use crate::query::QueryIgnoreRules;
use crate::query::QueryPlan;
use crate::query::QueryRowSink;
use crate::query::QueryRowStream;
use crate::query::load_and_query_drive_search_index;
use eyre::bail;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use tracing::info_span;

#[derive(Debug)]
pub struct DiskQueryExecutor {
    pub sync_dir: PathBuf,
    pub mft_files: Vec<(char, PathBuf)>,
    pub request: QueryPlan,
    pub ignore: QueryIgnoreBehavior,
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
                .map(|drive_letter| (drive_letter, sync_dir.join(format!("{drive_letter}.mft"))))
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
            ignore: QueryIgnoreBehavior::AutoDiscover,
        })
    }

    /// # Errors
    ///
    /// Returns an error if query parsing, scope resolution, or ignore discovery fails.
    pub fn stream(self) -> eyre::Result<QueryRowStream> {
        let _span = info_span!("query_execute").entered();
        let query_plan = Arc::new(self.request.clone());
        let drive_letters = self
            .mft_files
            .iter()
            .map(|(drive_letter, _)| *drive_letter)
            .collect::<Vec<_>>();
        let ignore_rules = {
            let _span = info_span!("query_prepare_filters").entered();
            match self.ignore {
                QueryIgnoreBehavior::AutoDiscover => Some(
                    QueryIgnoreRules::discover_for_drive_letters(&drive_letters, &self.sync_dir)?,
                ),
                QueryIgnoreBehavior::Disabled => None,
                QueryIgnoreBehavior::Custom(rules) => Some(rules),
            }
        };
        let filter = Arc::new(QueryFilter::new(&self.request, ignore_rules)?);
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let sink = QueryRowSink::new(tx);
        let drive_count = self.mft_files.len();
        let sync_dir = Arc::new(self.sync_dir);
        let include_deleted = self.request.include_deleted;
        let only_deleted = self.request.only_deleted;

        std::thread::spawn(move || {
            let _span = info_span!("query_disk_producers").entered();
            let mut handles = Vec::with_capacity(drive_count);
            for (drive_letter, _) in self.mft_files {
                let sink = sink.clone();
                let query_plan = Arc::clone(&query_plan);
                let filter = Arc::clone(&filter);
                let sync_dir = Arc::clone(&sync_dir);
                handles.push(std::thread::spawn(move || {
                    let _span = info_span!("query_drive_task").entered();
                    let result = load_and_query_drive_search_index(
                        drive_letter,
                        &sync_dir,
                        &query_plan,
                        include_deleted,
                        only_deleted,
                    );
                    match result {
                        Ok(result) => {
                            for row in result.matched_rows {
                                let Some(row) = filter.classify_and_match(row) else {
                                    continue;
                                };
                                if sink.blocking_send(row).is_err() {
                                    return;
                                }
                            }
                            info!(
                                drive = %drive_letter,
                                loaded_rows = result.loaded_rows,
                                "Disk query drive completed"
                            );
                        }
                        Err(error) => {
                            let _ = sink.blocking_send_error(error);
                        }
                    }
                }));
            }
            for handle in handles {
                let _ = handle.join();
            }
        });

        Ok(QueryRowStream::Local(rx))
    }
}
