use crate::machine::config::load_sync_dir_from_config;
use crate::machine::config::published_drive_paths;
use crate::query::ControlFlow;
use crate::query::QueryFilterRules;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryRowFilter;
use crate::query::QueryRowSink;
use crate::query::QueryRowStream;
use crate::query::QueryRuntime;
use crate::query::search_index_query::mapped_search_index_has_rows;
use crate::query::search_index_query::merge_rows;
use crate::query::visit_parsed_search_index_rows;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use eyre::Context;
use eyre::ContextCompat;
use eyre::ensure;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum QuerySessionBackend {
    PublishedIndexOnly,
    DaemonRpc,
}

/// A persistent in-process query facade.
///
/// Use `QuerySession` when a caller intends to issue multiple queries against
/// the same published index cache in one process and wants to reuse cached
/// drive state across those requests. For one-shot backend-agnostic query
/// execution, prefer `QueryRuntime`.
#[derive(Debug)]
pub struct QuerySession {
    backend: QuerySessionBackend,
    sync_dir: PathBuf,
    published_index_cache: HashMap<char, CachedPublishedDriveQuery>,
}

#[derive(Debug)]
pub(crate) struct SpawnedQuerySessionStream {
    pub stream: QueryRowStream,
    pub query_join: std::thread::JoinHandle<eyre::Result<()>>,
}

#[derive(Debug)]
struct CachedPublishedDriveQuery {
    drive: char,
    base_index: MappedSearchIndex,
    overlay_index: Option<MappedSearchIndex>,
}

impl QuerySession {
    /// # Errors
    ///
    /// Returns an error if the local sync directory cannot be loaded from
    /// config.
    pub fn published_index_only() -> eyre::Result<Self> {
        Ok(Self {
            backend: QuerySessionBackend::PublishedIndexOnly,
            sync_dir: load_sync_dir_from_config()?,
            published_index_cache: HashMap::new(),
        })
    }

    /// # Errors
    ///
    /// Returns an error if the local sync directory cannot be loaded from
    /// config.
    pub fn daemon_rpc() -> eyre::Result<Self> {
        Ok(Self {
            backend: QuerySessionBackend::DaemonRpc,
            sync_dir: load_sync_dir_from_config()?,
            published_index_cache: HashMap::new(),
        })
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend cannot answer the query.
    pub fn collect_rows(&mut self, query_plan: QueryPlan) -> eyre::Result<Vec<QueryResultRow>> {
        self.collect_rows_with_cancel(query_plan, None)
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend cannot answer the query.
    /// Cancellation is best-effort and returns the rows collected so far.
    pub fn collect_rows_with_cancel(
        &mut self,
        query_plan: QueryPlan,
        cancel: Option<&AtomicBool>,
    ) -> eyre::Result<Vec<QueryResultRow>> {
        let mut rows = Vec::new();
        self.visit_rows_with_cancel(query_plan, cancel, |row| {
            rows.push(row);
            Ok(ControlFlow::Continue)
        })?;
        Ok(rows)
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend cannot answer the query.
    /// Cancellation is best-effort and returns after the rows visited so far.
    pub fn visit_rows_with_cancel(
        &mut self,
        query_plan: QueryPlan,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow>,
    ) -> eyre::Result<()> {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Ok(());
        }

        let limit = query_plan.limit.get();
        let mut visited_rows = 0_usize;
        let mut visit_with_limit = |row| {
            visited_rows += 1;
            let control_flow = visit(row)?;
            if control_flow == ControlFlow::Break {
                return Ok(ControlFlow::Break);
            }
            if let Some(limit) = limit
                && visited_rows >= limit
            {
                return Ok(ControlFlow::Break);
            }
            Ok(ControlFlow::Continue)
        };

        match self.backend {
            QuerySessionBackend::PublishedIndexOnly => {
                self.visit_published_index_rows(&query_plan, cancel, &mut visit_with_limit)
            }
            QuerySessionBackend::DaemonRpc => QueryRuntime::daemon_rpc()
                .prepare_stream(query_plan)?
                .visit_rows(&mut visit_with_limit),
        }
    }

    /// # Errors
    ///
    /// Returns an error if this session backend does not support direct local
    /// stream production.
    pub(crate) fn spawn_stream(
        self,
        query_plan: QueryPlan,
        cancel: Arc<AtomicBool>,
    ) -> eyre::Result<SpawnedQuerySessionStream> {
        match self.backend {
            QuerySessionBackend::PublishedIndexOnly => {
                let (tx, rx) = tokio::sync::mpsc::channel(256);
                let sink = QueryRowSink::new(tx);
                let query_join = std::thread::spawn(move || {
                    let mut session = self;
                    session.visit_rows_with_cancel(query_plan, Some(cancel.as_ref()), |row| {
                        Ok(if sink.blocking_send(row).is_ok() {
                            ControlFlow::Continue
                        } else {
                            ControlFlow::Break
                        })
                    })?;
                    Ok(())
                });
                Ok(SpawnedQuerySessionStream {
                    stream: QueryRowStream::Local(rx),
                    query_join,
                })
            }
            QuerySessionBackend::DaemonRpc => eyre::bail!(
                "streaming row production is only supported for published-index query sessions"
            ),
        }
    }

    fn visit_published_index_rows(
        &mut self,
        query_plan: &QueryPlan,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow>,
    ) -> eyre::Result<()> {
        let drive_letters = query_plan
            .drive_letter_pattern
            .clone()
            .into_drive_letters()?;
        let filter_rules = QueryFilterRules::discover_for_drive_letters(
            &drive_letters,
            &self.sync_dir,
            query_plan.profile.as_deref(),
        )?;
        let filter = QueryRowFilter::new(query_plan, Some(filter_rules))?;

        for &drive in &drive_letters {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                break;
            }

            let control_flow = self
                .cached_drive(drive)?
                .visit_rows_with_cancel(query_plan, &filter, cancel, &mut visit)
                .wrap_err_with(|| {
                    format!("failed querying cached published index for drive {drive}")
                })?;
            if control_flow == ControlFlow::Break {
                break;
            }
        }

        Ok(())
    }

    fn cached_drive(&mut self, drive: char) -> eyre::Result<&CachedPublishedDriveQuery> {
        if !self.published_index_cache.contains_key(&drive) {
            let cache = CachedPublishedDriveQuery::load(drive, &self.sync_dir)?;
            self.published_index_cache.insert(drive, cache);
        }

        self.published_index_cache
            .get(&drive)
            .wrap_err_with(|| format!("missing cached published query state for drive {drive}"))
    }
}

impl CachedPublishedDriveQuery {
    fn load(drive: char, sync_dir: &std::path::Path) -> eyre::Result<Self> {
        let paths = published_drive_paths(sync_dir, drive);
        ensure!(
            paths.base_index_path.is_file(),
            "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
            paths.base_index_path.display(),
            drive
        );

        let base_index = MappedSearchIndex::open(&paths.base_index_path).wrap_err_with(|| {
            format!(
                "Failed loading base search index for drive {} from {}",
                drive,
                paths.base_index_path.display()
            )
        })?;

        let overlay_index = if paths.overlay_index_path.is_file() {
            let overlay =
                MappedSearchIndex::open(&paths.overlay_index_path).wrap_err_with(|| {
                    format!(
                        "Failed loading overlay search index for drive {} from {}",
                        drive,
                        paths.overlay_index_path.display()
                    )
                })?;
            mapped_search_index_has_rows(&overlay).then_some(overlay)
        } else {
            None
        };

        Ok(Self {
            drive,
            base_index,
            overlay_index,
        })
    }

    fn visit_rows_with_cancel(
        &self,
        query_plan: &QueryPlan,
        filter: &QueryRowFilter,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow>,
    ) -> eyre::Result<ControlFlow> {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Ok(ControlFlow::Break);
        }

        if let Some(overlay_index) = self.overlay_index.as_ref() {
            let mut result = Self::collect_index_rows(&self.base_index, query_plan, cancel)
                .wrap_err_with(|| {
                    format!("failed querying cached base index for drive {}", self.drive)
                })?;
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                return Ok(ControlFlow::Break);
            }
            let overlay_rows = Self::collect_index_rows(overlay_index, query_plan, cancel)
                .wrap_err_with(|| {
                    format!(
                        "failed querying cached overlay index for drive {}",
                        self.drive
                    )
                })?;
            result = merge_rows(result, overlay_rows);
            for row in result {
                if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                    return Ok(ControlFlow::Break);
                }
                let Some(row) = filter.classify_and_match(row) else {
                    continue;
                };
                if visit(row)? == ControlFlow::Break {
                    return Ok(ControlFlow::Break);
                }
            }
            return Ok(ControlFlow::Continue);
        }

        Self::visit_index_rows(&self.base_index, query_plan, filter, cancel, &mut visit)
            .wrap_err_with(|| format!("failed querying cached base index for drive {}", self.drive))
    }

    fn visit_index_rows(
        mapped: &MappedSearchIndex,
        query_plan: &QueryPlan,
        filter: &QueryRowFilter,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow>,
    ) -> eyre::Result<ControlFlow> {
        let parsed_index = SearchIndexBytes::new(mapped.bytes()).parse_trusted_for_query()?;
        let (_loaded_rows, control_flow) = visit_parsed_search_index_rows(
            &parsed_index,
            query_plan,
            query_plan.include_deleted,
            query_plan.only_deleted,
            |row| {
                if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                    return Ok(ControlFlow::Break);
                }
                let Some(row) = filter.classify_and_match(row) else {
                    return Ok(ControlFlow::Continue);
                };
                visit(row)
            },
        )?;
        Ok(control_flow)
    }

    fn collect_index_rows(
        mapped: &MappedSearchIndex,
        query_plan: &QueryPlan,
        cancel: Option<&AtomicBool>,
    ) -> eyre::Result<Vec<QueryResultRow>> {
        let mut rows = Vec::new();
        let (_loaded_rows, _control_flow) = visit_parsed_search_index_rows(
            &SearchIndexBytes::new(mapped.bytes()).parse_trusted_for_query()?,
            query_plan,
            query_plan.include_deleted,
            query_plan.only_deleted,
            |row| {
                if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                    return Ok(ControlFlow::Break);
                }
                rows.push(row);
                Ok(ControlFlow::Continue)
            },
        )?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::QuerySession;
    use super::SpawnedQuerySessionStream;
    use crate::machine::config::published_drive_paths;
    use crate::query::ControlFlow;
    use crate::query::QueryPlan;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    fn write_drive_index(
        temp_dir: &TempDir,
        drive: char,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<()> {
        let paths = published_drive_paths(temp_dir.path(), drive);
        std::fs::create_dir_all(
            paths
                .base_index_path
                .parent()
                .expect("published index path should have a parent"),
        )?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, 123, rows.len() as u64),
            rows,
        )?
        .write_to_path(&paths.base_index_path)?;
        Ok(())
    }

    #[test]
    fn published_session_reuses_cached_drive_entries() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml"),
                has_deleted_entries: false,
            }],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::PublishedIndexOnly,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };

        let first = session.collect_rows(QueryPlan::new("Cargo.toml"))?;
        let second = session.collect_rows(QueryPlan::new("Repos"))?;

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(session.published_index_cache.len(), 1);

        Ok(())
    }

    #[test]
    fn daemon_session_still_uses_runtime_backend_selection() -> eyre::Result<()> {
        let session = QuerySession::daemon_rpc()?;
        assert_eq!(session.backend, super::QuerySessionBackend::DaemonRpc);
        Ok(())
    }

    #[test]
    fn published_session_returns_no_rows_when_cancelled_before_query() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml"),
                has_deleted_entries: false,
            }],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::PublishedIndexOnly,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let cancel = std::sync::atomic::AtomicBool::new(true);

        let rows = session.collect_rows_with_cancel(QueryPlan::new("Cargo.toml"), Some(&cancel))?;

        assert!(rows.is_empty());
        Ok(())
    }

    #[test]
    fn published_session_count_rows_matches_visited_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[
                SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\Cargo.toml"),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\package.json"),
                    has_deleted_entries: false,
                },
            ],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::PublishedIndexOnly,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let mut visited = Vec::new();
        session.visit_rows_with_cancel(QueryPlan::new("Repos"), None, |row| {
            visited.push(row.path.to_string());
            Ok(ControlFlow::Continue)
        })?;

        let count = {
            let mut count = 0_usize;
            session.visit_rows_with_cancel(QueryPlan::new("Repos"), None, |_row| {
                count += 1;
                Ok(ControlFlow::Continue)
            })?;
            eyre::Ok(count)
        }?;

        assert_eq!(count, visited.len());
        assert_eq!(count, 2);
        Ok(())
    }

    #[test]
    fn published_session_visit_rows_respects_query_limit() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[
                SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\Cargo.toml"),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\package.json"),
                    has_deleted_entries: false,
                },
            ],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::PublishedIndexOnly,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let mut plan = QueryPlan::new("Repos");
        plan.limit = 1_usize.into();
        let mut visited = 0_usize;

        session.visit_rows_with_cancel(plan, None, |_row| {
            visited += 1;
            Ok(ControlFlow::Continue)
        })?;

        assert_eq!(visited, 1);
        Ok(())
    }

    #[test]
    fn published_session_spawn_stream_emits_matching_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml"),
                has_deleted_entries: false,
            }],
        )?;

        let session = QuerySession {
            backend: super::QuerySessionBackend::PublishedIndexOnly,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let plan = QueryPlan::new("Cargo.toml");
        let SpawnedQuerySessionStream { stream, query_join } =
            session.spawn_stream(plan.clone(), Arc::new(AtomicBool::new(false)))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let rows = runtime.block_on(stream.collect_filtered_limit(plan.limit))?;
        query_join
            .join()
            .map_err(|join_error| eyre::eyre!("query thread panicked: {join_error:?}"))??;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path.as_str(), r"C:\Repos\app\Cargo.toml");
        Ok(())
    }
}
