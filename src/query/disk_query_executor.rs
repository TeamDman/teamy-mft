use crate::query::QueryFilter;
use crate::query::QueryIgnoreBehavior;
use crate::query::QueryIgnoreRules;
use crate::query::QueryPlan;
use crate::query::QueryRequestSpec;
use crate::query::QueryRowSink;
use crate::query::QueryRowStream;
use crate::query::load_and_query_drive_search_index;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use tracing::info_span;

#[derive(Debug)]
pub struct DiskQueryExecutor {
    sync_dir: PathBuf,
    mft_files: Vec<(char, PathBuf)>,
    spec: QueryRequestSpec,
    ignore: QueryIgnoreBehavior,
}

impl DiskQueryExecutor {
    #[must_use]
    pub fn new(
        sync_dir: &Path,
        drive_letters: Vec<char>,
        spec: QueryRequestSpec,
        ignore: QueryIgnoreBehavior,
    ) -> Self {
        let mft_files = {
            let _span = info_span!("discover_mft_files").entered();
            drive_letters
                .into_iter()
                .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
                .filter(|(_, p)| p.is_file())
                .collect()
        };
        Self {
            sync_dir: sync_dir.to_path_buf(),
            mft_files,
            spec,
            ignore,
        }
    }

    /// # Errors
    ///
    /// Returns an error if query parsing, scope resolution, or ignore discovery fails.
    pub fn stream(self) -> eyre::Result<QueryRowStream> {
        let _span = info_span!("query_execute").entered();
        let query_plan = Arc::new(QueryPlan::parse_inputs(&self.spec.query)?);
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
        let filter = Arc::new(QueryFilter::new(&self.spec, ignore_rules)?);
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let sink = QueryRowSink::new(tx);
        let drive_count = self.mft_files.len();
        let sync_dir = Arc::new(self.sync_dir);
        let include_deleted = self.spec.include_deleted;
        let only_deleted = self.spec.only_deleted;

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
