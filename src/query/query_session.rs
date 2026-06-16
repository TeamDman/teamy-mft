use crate::machine::config::load_sync_dir_from_config;
use crate::machine::config::published_drive_paths;
use crate::query::QueryFilterRules;
use crate::query::Pathlike;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryRowFilter;
use crate::query::QueryRuntime;
use crate::query::search_index_query::mapped_search_index_has_rows;
use crate::query::search_index_query::visit_matching_parsed_row_indices;
use crate::query::visit_parsed_search_index_rows;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::ParsedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use eyre::Context;
use eyre::ContextCompat;
use eyre::ensure;
use tracing::info_span;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use super::query_runtime::QueryRowVisitor;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum QuerySessionBackend {
    Local,
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
    pub fn local() -> eyre::Result<Self> {
        Ok(Self {
            backend: QuerySessionBackend::Local,
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
    pub fn visit_rows(
        &mut self,
        query_plan: QueryPlan,
        visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<()> {
        self.visit_rows_with_cancel(query_plan, None, visit)
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend cannot answer the query.
    /// Cancellation is best-effort and returns after the rows visited so far.
    pub fn visit_rows_with_cancel(
        &mut self,
        query_plan: QueryPlan,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<()> {
        self.visit_rows_with_cancel_dyn(query_plan, cancel, &mut visit)
    }

    pub(crate) fn visit_rows_with_cancel_dyn(
        &mut self,
        query_plan: QueryPlan,
        cancel: Option<&AtomicBool>,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
        let _guard = info_span!("visit_rows_with_cancel_dyn").entered();
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Ok(());
        }

        let limit = query_plan.limit.get();
        let mut visited_rows = 0_usize;
        let mut visit_with_limit = |row| {
            visited_rows += 1;
            match visit(row)? {
                ControlFlow::Continue(()) => {
                    if let Some(limit) = limit
                        && visited_rows >= limit
                    {
                        return Ok(ControlFlow::Break(()));
                    }
                    Ok(ControlFlow::Continue(()))
                }
                ControlFlow::Break(()) => {
                    Ok(ControlFlow::Break(()))
                }
            }
        };

        match self.backend {
            QuerySessionBackend::Local => {
                self.visit_published_index_rows(&query_plan, cancel, &mut visit_with_limit)?;
            }
            QuerySessionBackend::DaemonRpc => {
                QueryRuntime::daemon_rpc().visit_rows_dyn(query_plan, &mut visit_with_limit)?;
            }
        }

        Ok(())
    }

    fn visit_published_index_rows(
        &mut self,
        query_plan: &QueryPlan,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<()> {
        let _guard = info_span!("visit_published_index_rows").entered();
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
            let _guard = info_span!("visit_drive_rows", drive = %drive).entered();
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                break;
            }

            let control_flow = self
                .cached_drive(drive)?
                .visit_rows_with_cancel(query_plan, &filter, cancel, &mut visit)
                .wrap_err_with(|| {
                    format!("failed querying cached published index for drive {drive}")
                })?;
            if control_flow == ControlFlow::Break(()) {
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
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<ControlFlow<()>> {
        let _guard = info_span!("visit_rows_with_cancel", drive = %self.drive).entered();
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Ok(ControlFlow::Break(()));
        }

        if let Some(overlay_index) = self.overlay_index.as_ref() {
            let base_parsed_index = SearchIndexBytes::new(self.base_index.bytes())
                .parse_trusted_for_query()
                .wrap_err_with(|| {
                    format!("failed preparing cached base index for drive {}", self.drive)
                })?;
            let overlay_parsed_index = SearchIndexBytes::new(overlay_index.bytes())
                .parse_trusted_for_query()
                .wrap_err_with(|| {
                    format!("failed preparing cached overlay index for drive {}", self.drive)
                })?;
            let mut base_rows = Self::collect_matching_row_refs(
                &base_parsed_index,
                query_plan,
                cancel,
            )
            .wrap_err_with(|| {
                format!("failed querying cached base index for drive {}", self.drive)
            })?;
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                return Ok(ControlFlow::Break(()));
            }
            let mut overlay_rows = Self::collect_matching_row_refs(
                &overlay_parsed_index,
                query_plan,
                cancel,
            )
                .wrap_err_with(|| {
                    format!(
                        "failed querying cached overlay index for drive {}",
                        self.drive
                    )
                })?;

            base_rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));
            overlay_rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));

            let mut base_offset = 0_usize;
            let mut overlay_offset = 0_usize;
            while base_offset < base_rows.len() || overlay_offset < overlay_rows.len() {
                if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                    return Ok(ControlFlow::Break(()));
                }
                let row = match (base_rows.get(base_offset), overlay_rows.get(overlay_offset)) {
                    (Some(base_row), Some(overlay_row)) => {
                        if overlay_row.path <= base_row.path {
                            if overlay_row.path == base_row.path {
                                base_offset += 1;
                            }
                            overlay_offset += 1;
                            Self::materialize_row(&overlay_parsed_index, overlay_row.row_index)?
                        } else {
                            base_offset += 1;
                            Self::materialize_row(&base_parsed_index, base_row.row_index)?
                        }
                    }
                    (Some(base_row), None) => {
                        base_offset += 1;
                        Self::materialize_row(&base_parsed_index, base_row.row_index)?
                    }
                    (None, Some(overlay_row)) => {
                        overlay_offset += 1;
                        Self::materialize_row(&overlay_parsed_index, overlay_row.row_index)?
                    }
                    (None, None) => break,
                };
                let Some(row) = filter.classify_and_match(row) else {
                    continue;
                };
                if visit(row)? == ControlFlow::Break(()) {
                    return Ok(ControlFlow::Break(()));
                }
            }
            return Ok(ControlFlow::Continue(()));
        }

        Self::visit_index_rows(&self.base_index, query_plan, filter, cancel, &mut visit)
            .wrap_err_with(|| format!("failed querying cached base index for drive {}", self.drive))
    }

    fn visit_index_rows(
        mapped: &MappedSearchIndex,
        query_plan: &QueryPlan,
        filter: &QueryRowFilter,
        cancel: Option<&AtomicBool>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<ControlFlow<()>> {
        let parsed_index = SearchIndexBytes::new(mapped.bytes()).parse_trusted_for_query()?;
        let (_loaded_rows, control_flow) = visit_parsed_search_index_rows(
            &parsed_index,
            query_plan,
            query_plan.include_deleted,
            query_plan.only_deleted,
            |row| {
                if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                    return Ok(ControlFlow::Break(()));
                }
                let Some(row) = filter.classify_and_match(row) else {
                    return Ok(ControlFlow::Continue(()));
                };
                visit(row)
            },
        )?;
        Ok(control_flow)
    }

    fn collect_matching_row_refs<'a>(
        parsed_index: &'a ParsedSearchIndex<'a>,
        query_plan: &QueryPlan,
        cancel: Option<&AtomicBool>,
    ) -> eyre::Result<Vec<MatchingRowRef>> {
        let mut rows = Vec::new();
        let (_loaded_rows, _control_flow) = visit_matching_parsed_row_indices(
            parsed_index,
            query_plan,
            query_plan.include_deleted,
            query_plan.only_deleted,
            |row_index| {
                if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                    return Ok(ControlFlow::Break(()));
                }
                let row = parsed_index.row_view(row_index as usize)?;
                rows.push(MatchingRowRef {
                    row_index,
                    path: row.path(),
                });
                Ok(ControlFlow::Continue(()))
            },
        )?;
        Ok(rows)
    }

    fn materialize_row(
        parsed_index: &ParsedSearchIndex<'_>,
        row_index: u32,
    ) -> eyre::Result<QueryResultRow> {
        let row = parsed_index.row_view(row_index as usize)?;
        Ok(QueryResultRow {
            path: row.path(),
            has_deleted_entries: row.has_deleted_entries,
            is_filtered: false,
        })
    }
}

struct MatchingRowRef {
    row_index: u32,
    path: Pathlike,
}

#[cfg(test)]
mod tests {
    use super::QuerySession;
    use crate::machine::config::published_drive_paths;
    use crate::query::QueryPlan;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use crate::windows_utils::storage::DriveLetterPattern;
    use std::ops::ControlFlow;
    use tempfile::TempDir;

    fn collect_visited_paths(
        session: &mut QuerySession,
        query_plan: QueryPlan,
    ) -> eyre::Result<Vec<String>> {
        let mut rows = Vec::new();
        session.visit_rows(query_plan, |row| {
            rows.push(row.path.to_string());
            Ok(ControlFlow::Continue(()))
        })?;
        Ok(rows)
    }

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

    fn write_overlay_drive_index(
        temp_dir: &TempDir,
        drive: char,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<()> {
        let paths = published_drive_paths(temp_dir.path(), drive);
        std::fs::create_dir_all(
            paths
                .overlay_index_path
                .parent()
                .expect("published overlay index path should have a parent"),
        )?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, 123, rows.len() as u64),
            rows,
        )?
        .write_to_path(&paths.overlay_index_path)?;
        Ok(())
    }

    fn fixture_drive_plan(pattern: &str) -> QueryPlan {
        QueryPlan {
            drive_letter_pattern: DriveLetterPattern(String::from("C")),
            ..QueryPlan::new(pattern)
        }
    }

    #[test]
    fn published_session_reuses_cached_drive_entries() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                has_deleted_entries: false,
            }],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::Local,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };

        let first = collect_visited_paths(&mut session, fixture_drive_plan("Cargo.toml"))?;
        let second = collect_visited_paths(&mut session, fixture_drive_plan("Repos"))?;

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
                path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                has_deleted_entries: false,
            }],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::Local,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let mut visited = 0_usize;

        session.visit_rows_with_cancel(
            fixture_drive_plan("Cargo.toml"),
            Some(&cancel),
            |_row| -> eyre::Result<ControlFlow<()>> {
                visited += 1;
                Ok(ControlFlow::Continue(()))
            },
        )?;

        assert_eq!(visited, 0);
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
                    path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\package.json").into(),
                    has_deleted_entries: false,
                },
            ],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::Local,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let mut visited = Vec::new();
        session.visit_rows_with_cancel(
            fixture_drive_plan("Repos"),
            None,
            |row| -> eyre::Result<ControlFlow<()>> {
                visited.push(row.path.to_string());
                Ok(ControlFlow::Continue(()))
            },
        )?;

        let count = {
            let mut count = 0_usize;
            session.visit_rows_with_cancel(
                fixture_drive_plan("Repos"),
                None,
                |_row| -> eyre::Result<ControlFlow<()>> {
                    count += 1;
                    Ok(ControlFlow::Continue(()))
                },
            )?;
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
                    path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\package.json").into(),
                    has_deleted_entries: false,
                },
            ],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::Local,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let mut plan = fixture_drive_plan("Repos");
        plan.limit = 1_usize.into();
        let mut visited = 0_usize;

        session.visit_rows_with_cancel(
            plan,
            None,
            |_row| -> eyre::Result<ControlFlow<()>> {
                visited += 1;
                Ok(ControlFlow::Continue(()))
            },
        )?;

        assert_eq!(visited, 1);
        Ok(())
    }

    #[test]
    fn published_session_visit_rows_emits_matching_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                has_deleted_entries: false,
            }],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::Local,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let rows = collect_visited_paths(&mut session, fixture_drive_plan("Cargo.toml"))?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], r"C:\Repos\app\Cargo.toml");
        Ok(())
    }

    #[test]
    fn published_session_overlay_rows_override_cached_base_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                has_deleted_entries: false,
            }],
        )?;
        write_overlay_drive_index(
            &temp_dir,
            'C',
            &[SearchIndexPathRow {
                path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                has_deleted_entries: true,
            }],
        )?;

        let mut session = QuerySession {
            backend: super::QuerySessionBackend::Local,
            sync_dir: temp_dir.path().to_path_buf(),
            published_index_cache: std::collections::HashMap::new(),
        };
        let mut plan = fixture_drive_plan("Cargo.toml");
        plan.include_deleted = true;
        let mut rows = Vec::new();

        session.visit_rows(plan, |row| {
            rows.push(row);
            Ok(ControlFlow::Continue(()))
        })?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path.to_string(), r"C:\Repos\app\Cargo.toml");
        assert!(rows[0].has_deleted_entries);
        Ok(())
    }
}
