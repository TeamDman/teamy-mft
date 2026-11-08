use super::query_runtime::QueryRowVisitor;
use crate::domain::DriveLetter;
use crate::machine::DriveArtifactInfo;
use crate::machine::MachineConfig;
use crate::query::Pathlike;
use crate::query::QueryFilterRules;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryRowFilter;
use crate::query::QueryRuntime;
use crate::query::QueryScope;
use crate::query::resolve_query_scopes;
use crate::query::search_index_query::mapped_search_index_has_rows;
use crate::query::search_index_query::visit_matching_parsed_row_indices;
use crate::query::visit_parsed_search_index_rows;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::ParsedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use eyre::Context;
use eyre::ContextCompat;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use teamy_cancellation::CancellationScope;
use tracing::info_span;

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
    published_index_cache: HashMap<DriveLetter, CachedPublishedDriveQuery>,
}

#[derive(Debug)]
struct CachedPublishedDriveQuery {
    drive: DriveLetter,
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
            sync_dir: MachineConfig::load()?.sync_dir.into_inner(),
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
            sync_dir: MachineConfig::load()?.sync_dir.into_inner(),
            published_index_cache: HashMap::new(),
        })
    }

    /// # Errors
    ///
    /// Returns an error if the configured backend cannot answer the query.
    pub fn visit_rows(
        &mut self,
        query_plan: QueryPlan,
        cancel: &CancellationScope,
        visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<()> {
        let mut visit = visit;
        self.visit_rows_dyn(query_plan, cancel, &mut visit)
    }

    pub(crate) fn visit_rows_dyn(
        &mut self,
        query_plan: QueryPlan,
        cancel: &CancellationScope,
        visit: &mut QueryRowVisitor<'_>,
    ) -> eyre::Result<()> {
        let _guard = info_span!("visit_rows_dyn").entered();
        if cancel.is_cancelled() {
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
                ControlFlow::Break(()) => Ok(ControlFlow::Break(())),
            }
        };

        match self.backend {
            QuerySessionBackend::Local => {
                self.visit_published_index_rows(&query_plan, cancel, &mut visit_with_limit)?;
            }
            QuerySessionBackend::DaemonRpc => {
                QueryRuntime::daemon_rpc().visit_rows_dyn(
                    query_plan,
                    cancel,
                    &mut visit_with_limit,
                )?;
            }
        }

        Ok(())
    }

    fn visit_published_index_rows(
        &mut self,
        query_plan: &QueryPlan,
        cancel: &CancellationScope,
        visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<()> {
        let _guard = info_span!("visit_published_index_rows").entered();
        let scopes = resolve_query_scopes(&query_plan.r#in)?;
        let drive_letters = query_plan
            .drive_letter_pattern
            .into_drive_letters_for_scope_roots(scopes.iter().map(|scope| scope.root.as_path()))?;
        self.visit_selected_published_index_rows(query_plan, cancel, drive_letters, visit)
    }

    // Keep OS drive enumeration separate so selection policy can be exercised
    // with tiny cached fixtures, independently of the machine's mounted drives.
    fn visit_selected_published_index_rows(
        &mut self,
        query_plan: &QueryPlan,
        cancel: &CancellationScope,
        drive_letters: Vec<DriveLetter>,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<()> {
        let skip_missing =
            query_plan.drive_letter_pattern.as_ref().trim() == "*" && query_plan.r#in.is_empty();
        let mut selected_drives = Vec::with_capacity(drive_letters.len());
        for drive in drive_letters {
            if skip_missing && !self.published_index_cache.contains_key(&drive) {
                let path = DriveArtifactInfo::new(&self.sync_dir, drive).base_index_path;
                match std::fs::metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        tracing::warn!(drive = %drive, index_path = %path.display(),
                            "Skipping drive without a search index for wildcard query");
                        continue;
                    }
                    Err(error) => {
                        return Err(error).wrap_err_with(|| {
                            format!(
                                "Failed inspecting search index for drive {drive} at {}",
                                path.display()
                            )
                        });
                    }
                    Ok(_) => {}
                }
            }
            selected_drives.push(drive);
        }
        let drive_letters = selected_drives;
        let filter_rules = QueryFilterRules::discover_for_drive_letters(
            &drive_letters,
            &self.sync_dir,
            query_plan.profile.as_deref(),
        )?;
        let filter = QueryRowFilter::new(query_plan, Some(filter_rules))?;

        for &drive in &drive_letters {
            let _guard = info_span!("visit_drive_rows", drive = %drive).entered();
            if cancel.is_cancelled() {
                break;
            }

            let control_flow = self
                .cached_drive(drive)?
                .visit_rows(query_plan, &filter, cancel, &mut visit)
                .wrap_err_with(|| {
                    format!("failed querying cached published index for drive {drive}")
                })?;
            if control_flow == ControlFlow::Break(()) {
                break;
            }
        }

        Ok(())
    }

    fn cached_drive(&mut self, drive: DriveLetter) -> eyre::Result<&CachedPublishedDriveQuery> {
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
    fn load(drive: DriveLetter, sync_dir: &std::path::Path) -> eyre::Result<Self> {
        let paths = DriveArtifactInfo::new(sync_dir, drive);
        paths.require_base_search_index()?;

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

    fn visit_rows(
        &self,
        query_plan: &QueryPlan,
        filter: &QueryRowFilter,
        cancel: &CancellationScope,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<ControlFlow<()>> {
        let _guard = info_span!("visit_rows", drive = %self.drive).entered();
        if cancel.is_cancelled() {
            return Ok(ControlFlow::Break(()));
        }

        if let Some(overlay_index) = self.overlay_index.as_ref() {
            let base_parsed_index = SearchIndexBytes::new(self.base_index.bytes())
                .parse_trusted_for_query()
                .wrap_err_with(|| {
                    format!(
                        "failed preparing cached base index for drive {}",
                        self.drive
                    )
                })?;
            let overlay_parsed_index = SearchIndexBytes::new(overlay_index.bytes())
                .parse_trusted_for_query()
                .wrap_err_with(|| {
                    format!(
                        "failed preparing cached overlay index for drive {}",
                        self.drive
                    )
                })?;
            let mut base_rows = Self::collect_matching_row_refs(
                &base_parsed_index,
                query_plan,
                filter.scopes(),
                cancel,
            )
            .wrap_err_with(|| {
                format!("failed querying cached base index for drive {}", self.drive)
            })?;
            if cancel.is_cancelled() {
                return Ok(ControlFlow::Break(()));
            }
            let mut overlay_rows = Self::collect_matching_row_refs(
                &overlay_parsed_index,
                query_plan,
                filter.scopes(),
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
                if cancel.is_cancelled() {
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
        cancel: &CancellationScope,
        mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<()>>,
    ) -> eyre::Result<ControlFlow<()>> {
        let parsed_index = SearchIndexBytes::new(mapped.bytes()).parse_trusted_for_query()?;
        let (_loaded_rows, control_flow) = visit_parsed_search_index_rows(
            &parsed_index,
            query_plan,
            filter.scopes(),
            query_plan.include_deleted,
            query_plan.only_deleted,
            |row| {
                if cancel.is_cancelled() {
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
        scopes: &[QueryScope],
        cancel: &CancellationScope,
    ) -> eyre::Result<Vec<MatchingRowRef>> {
        let mut rows = Vec::new();
        let (_loaded_rows, _control_flow) = visit_matching_parsed_row_indices(
            parsed_index,
            query_plan,
            scopes,
            query_plan.include_deleted,
            query_plan.only_deleted,
            |row_index| {
                if cancel.is_cancelled() {
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
    use crate::cancellation::CancellationToken;
    use crate::domain::DriveLetter;
    use crate::machine::DriveArtifactInfo;
    use crate::query::QueryPlan;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use crate::windows_utils::storage::DriveLetterPattern;
    use std::ops::ControlFlow;
    use tempfile::TempDir;
    use uom::si::information::byte;
    use uom::si::usize::Information;

    fn collect_visited_paths(
        session: &mut QuerySession,
        query_plan: QueryPlan,
    ) -> eyre::Result<Vec<String>> {
        CancellationToken::new().with_scope(|scope| {
            let mut rows = Vec::new();
            let cancel = scope.clone();
            session.visit_rows(query_plan, &cancel, |row| {
                rows.push(row.path.to_string());
                Ok(ControlFlow::Continue(()))
            })?;
            Ok(rows)
        })
    }

    fn write_drive_index(
        temp_dir: &TempDir,
        drive: DriveLetter,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<()> {
        let paths = DriveArtifactInfo::new(temp_dir.path(), drive);
        std::fs::create_dir_all(
            paths
                .base_index_path
                .parent()
                .expect("published index path should have a parent"),
        )?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, Information::new::<byte>(123), rows.len() as u64),
            rows,
        )?
        .write_to_path(&paths.base_index_path)?;
        Ok(())
    }

    fn write_overlay_drive_index(
        temp_dir: &TempDir,
        drive: DriveLetter,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<()> {
        let paths = DriveArtifactInfo::new(temp_dir.path(), drive);
        std::fs::create_dir_all(
            paths
                .overlay_index_path
                .parent()
                .expect("published overlay index path should have a parent"),
        )?;
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(drive, Information::new::<byte>(123), rows.len() as u64),
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
            DriveLetter::C,
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
        CancellationToken::new().with_scope(|scope| {
            let temp_dir = tempfile::tempdir()?;
            write_drive_index(
                &temp_dir,
                DriveLetter::C,
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
            let cancel = scope.clone();
            cancel.request_cancel("test cancellation");
            let mut visited = 0_usize;

            session.visit_rows(
                fixture_drive_plan("Cargo.toml"),
                &cancel,
                |_row| -> eyre::Result<ControlFlow<()>> {
                    visited += 1;
                    Ok(ControlFlow::Continue(()))
                },
            )?;

            assert_eq!(visited, 0);
            Ok(())
        })
    }

    #[test]
    fn published_session_count_rows_matches_visited_rows() -> eyre::Result<()> {
        CancellationToken::new().with_scope(|scope| {
            let temp_dir = tempfile::tempdir()?;
            write_drive_index(
                &temp_dir,
                DriveLetter::C,
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
            let cancel = scope.clone();
            session.visit_rows(
                fixture_drive_plan("Repos"),
                &cancel,
                |row| -> eyre::Result<ControlFlow<()>> {
                    visited.push(row.path.to_string());
                    Ok(ControlFlow::Continue(()))
                },
            )?;

            let count = {
                let mut count = 0_usize;
                session.visit_rows(
                    fixture_drive_plan("Repos"),
                    &cancel,
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
        })
    }

    #[test]
    fn published_session_visit_rows_respects_query_limit() -> eyre::Result<()> {
        CancellationToken::new().with_scope(|scope| {
            let temp_dir = tempfile::tempdir()?;
            write_drive_index(
                &temp_dir,
                DriveLetter::C,
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
            let cancel = scope.clone();

            session.visit_rows(plan, &cancel, |_row| -> eyre::Result<ControlFlow<()>> {
                visited += 1;
                Ok(ControlFlow::Continue(()))
            })?;

            assert_eq!(visited, 1);
            Ok(())
        })
    }

    #[test]
    fn published_session_visit_rows_emits_matching_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        write_drive_index(
            &temp_dir,
            DriveLetter::C,
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
        CancellationToken::new().with_scope(|scope| {
            let temp_dir = tempfile::tempdir()?;
            write_drive_index(
                &temp_dir,
                DriveLetter::C,
                &[SearchIndexPathRow {
                    path: String::from(r"C:\Repos\app\Cargo.toml").into(),
                    has_deleted_entries: false,
                }],
            )?;
            write_overlay_drive_index(
                &temp_dir,
                DriveLetter::C,
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
            let cancel = scope.clone();

            session.visit_rows(plan, &cancel, |row| {
                rows.push(row);
                Ok(ControlFlow::Continue(()))
            })?;

            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].path.to_string(), r"C:\Repos\app\Cargo.toml");
            assert!(rows[0].has_deleted_entries);
            Ok(())
        })
    }

    fn collect_selected_fixture_paths(
        temp_dir: &TempDir,
        plan: &QueryPlan,
        drives: Vec<DriveLetter>,
    ) -> eyre::Result<Vec<String>> {
        CancellationToken::new().with_scope(|scope| {
            let mut session = QuerySession {
                backend: super::QuerySessionBackend::Local,
                sync_dir: temp_dir.path().to_owned(),
                published_index_cache: std::collections::HashMap::new(),
            };
            let mut rows = Vec::new();
            session.visit_selected_published_index_rows(plan, &scope, drives, |row| {
                rows.push(row.path.to_string());
                Ok(ControlFlow::Continue(()))
            })?;
            Ok(rows)
        })
    }

    #[test]
    fn wildcard_query_ignores_inactive_orphaned_git_root_rules() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let review_config = temp.path().join("staged-review").join(".config");
        std::fs::create_dir_all(&review_config)?;
        let rule_path = review_config.join("sample.teamy-mft.teamy_mft_rules");
        std::fs::write(
            &rule_path,
            "DEFAULT RULE IS EXCLUDE\n\nORDER 2 INCLUDE {GIT_ROOT}\n",
        )?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[
                SearchIndexPathRow {
                    path: rule_path.to_string_lossy().into_owned().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: r"C:\Photos\20181226_145513.jpg".into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: r"C:\Recipes\hotsauce.txt".into(),
                    has_deleted_entries: false,
                },
            ],
        )?;

        for (query, expected) in [
            ("20181226_145513.jpg", r"C:\Photos\20181226_145513.jpg"),
            ("hotsauce", r"C:\Recipes\hotsauce.txt"),
        ] {
            let rows = collect_selected_fixture_paths(
                &temp,
                &QueryPlan::new(query),
                vec![DriveLetter::L, DriveLetter::C],
            )?;
            assert_eq!(rows, [expected]);
        }
        Ok(())
    }

    #[test]
    fn wildcard_query_ignores_inactive_profile_syntax_errors() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let rule_path = temp.path().join("broken.teamy-mft.teamy_mft_rules");
        std::fs::write(&rule_path, "THIS IS NOT A RULE\n")?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[
                SearchIndexPathRow {
                    path: rule_path.to_string_lossy().into_owned().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: r"C:\Recipes\hotsauce.txt".into(),
                    has_deleted_entries: false,
                },
            ],
        )?;
        let rows = collect_selected_fixture_paths(
            &temp,
            &QueryPlan::new("hotsauce"),
            vec![DriveLetter::C],
        )?;
        assert_eq!(rows, [r"C:\Recipes\hotsauce.txt"]);
        Ok(())
    }

    #[test]
    fn wildcard_query_ignores_inactive_profile_invalid_utf8() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let rule_path = temp.path().join("broken.teamy-mft.teamy_mft_rules");
        std::fs::write(&rule_path, [0xff, 0xfe])?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[
                SearchIndexPathRow {
                    path: rule_path.to_string_lossy().into_owned().into(),
                    has_deleted_entries: false,
                },
                SearchIndexPathRow {
                    path: r"C:\Recipes\hotsauce.txt".into(),
                    has_deleted_entries: false,
                },
            ],
        )?;
        let rows = collect_selected_fixture_paths(
            &temp,
            &QueryPlan::new("hotsauce"),
            vec![DriveLetter::C],
        )?;
        assert_eq!(rows, [r"C:\Recipes\hotsauce.txt"]);
        Ok(())
    }

    #[test]
    fn selected_orphaned_git_root_rules_keep_their_error() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let rule_path = temp.path().join("sample.teamy-mft.teamy_mft_rules");
        std::fs::write(
            &rule_path,
            "DEFAULT RULE IS EXCLUDE\n\nORDER 2 INCLUDE {GIT_ROOT}\n",
        )?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[SearchIndexPathRow {
                path: rule_path.to_string_lossy().into_owned().into(),
                has_deleted_entries: false,
            }],
        )?;
        let plan = QueryPlan {
            profile: Some("teamy-mft".into()),
            ..QueryPlan::new("20181226_145513.jpg")
        };
        let error = collect_selected_fixture_paths(&temp, &plan, vec![DriveLetter::C])
            .expect_err("selected invalid rules must still fail the query");
        let report = format!("{error:#}");
        assert!(report.contains("uses {GIT_ROOT} but no enclosing git root was found"));
        assert!(report.contains("sample.teamy-mft.teamy_mft_rules:3"));
        assert!(!report.contains("Fast query requires"), "{report}");
        assert!(!report.contains("Run `teamy-mft sync"), "{report}");
        Ok(())
    }

    #[test]
    fn global_orphaned_git_root_rules_keep_their_error() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let rule_path = temp.path().join("sample.teamy_mft_rules");
        std::fs::write(&rule_path, "INCLUDE {GIT_ROOT}\n")?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[SearchIndexPathRow {
                path: rule_path.to_string_lossy().into_owned().into(),
                has_deleted_entries: false,
            }],
        )?;
        let error = collect_selected_fixture_paths(
            &temp,
            &QueryPlan::new("hotsauce"),
            vec![DriveLetter::C],
        )
        .expect_err("invalid global rules must still fail the default query");
        let report = format!("{error:#}");
        assert!(report.contains("uses {GIT_ROOT} but no enclosing git root was found"));
        assert!(report.contains("sample.teamy_mft_rules:1"));
        assert!(!report.contains("Fast query requires"), "{report}");
        assert!(!report.contains("Run `teamy-mft sync"), "{report}");
        Ok(())
    }

    struct QueryWarningCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for QueryWarningCounter {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            if *event.metadata().level() == tracing::Level::WARN
                && event.metadata().target() == "teamy_mft::query::query_session"
            {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn wildcard_query_warns_for_missing_drive_and_returns_indexed_rows() -> eyre::Result<()> {
        use tracing_subscriber::layer::SubscriberExt;
        let temp = tempfile::tempdir()?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[SearchIndexPathRow {
                path: r"C:\Repos\Cargo.toml".into(),
                has_deleted_entries: false,
            }],
        )?;
        let warnings = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(QueryWarningCounter(std::sync::Arc::clone(&warnings)));
        let rows = tracing::subscriber::with_default(subscriber, || {
            collect_selected_fixture_paths(
                &temp,
                &QueryPlan::new("Cargo.toml"),
                vec![DriveLetter::L, DriveLetter::C],
            )
        })?;
        assert_eq!(rows, [r"C:\Repos\Cargo.toml"]);
        assert_eq!(warnings.load(std::sync::atomic::Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn explicit_drives_require_every_selected_index() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[SearchIndexPathRow {
                path: r"C:\Repos\Cargo.toml".into(),
                has_deleted_entries: false,
            }],
        )?;
        for pattern in ["L", "CL"] {
            let plan = QueryPlan {
                drive_letter_pattern: DriveLetterPattern(pattern.into()),
                ..QueryPlan::new("Cargo.toml")
            };
            let error = collect_selected_fixture_paths(
                &temp,
                &plan,
                plan.drive_letter_pattern.into_drive_letters()?,
            )
            .unwrap_err();
            assert!(error.to_string().contains("Fast query requires"));
        }
        Ok(())
    }

    #[test]
    fn wildcard_query_with_all_indexes_missing_warns_and_returns_no_rows() -> eyre::Result<()> {
        use tracing_subscriber::layer::SubscriberExt;
        let temp = tempfile::tempdir()?;
        let warnings = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(QueryWarningCounter(std::sync::Arc::clone(&warnings)));
        let rows = tracing::subscriber::with_default(subscriber, || {
            collect_selected_fixture_paths(
                &temp,
                &QueryPlan::new("Cargo.toml"),
                vec![DriveLetter::L, DriveLetter::M],
            )
        })?;
        assert!(rows.is_empty());
        assert_eq!(warnings.load(std::sync::atomic::Ordering::Relaxed), 2);
        Ok(())
    }

    #[test]
    fn scoped_wildcard_query_keeps_missing_index_error() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        let plan = QueryPlan {
            r#in: vec![temp.path().to_string_lossy().into_owned()],
            ..QueryPlan::new("Cargo.toml")
        };
        let drives = plan
            .drive_letter_pattern
            .into_drive_letters_for_scope_roots([temp.path()])?;
        let error = collect_selected_fixture_paths(&temp, &plan, drives).unwrap_err();
        assert!(error.to_string().contains("Fast query requires"));
        Ok(())
    }

    #[test]
    fn wildcard_query_does_not_skip_corrupt_or_nonfile_indexes() -> eyre::Result<()> {
        for directory in [false, true] {
            let temp = tempfile::tempdir()?;
            let path = DriveArtifactInfo::new(temp.path(), DriveLetter::L).base_index_path;
            if directory {
                std::fs::create_dir(&path)?;
            } else {
                std::fs::write(&path, b"corrupt index")?;
            }
            let error = collect_selected_fixture_paths(
                &temp,
                &QueryPlan::new("Cargo.toml"),
                vec![DriveLetter::L],
            )
            .expect_err("corrupt indexes and index directories should fail wildcard queries");
            let report = format!("{error:#}");
            assert!(!report.contains("Fast query requires"), "{report}");
            assert!(!report.contains("Run `teamy-mft sync"), "{report}");
        }
        Ok(())
    }

    #[test]
    fn wildcard_query_does_not_hide_corrupt_overlay_or_invalid_profile() -> eyre::Result<()> {
        let temp = tempfile::tempdir()?;
        write_drive_index(
            &temp,
            DriveLetter::C,
            &[SearchIndexPathRow {
                path: r"C:\Repos\Cargo.toml".into(),
                has_deleted_entries: false,
            }],
        )?;
        std::fs::write(
            DriveArtifactInfo::new(temp.path(), DriveLetter::C).overlay_index_path,
            b"corrupt overlay",
        )?;
        let _error = collect_selected_fixture_paths(
            &temp,
            &QueryPlan::new("Cargo.toml"),
            vec![DriveLetter::L, DriveLetter::C],
        )
        .expect_err("corrupt overlays should fail wildcard queries");
        let plan = QueryPlan {
            profile: Some("undiscovered-profile".into()),
            ..QueryPlan::new("Cargo.toml")
        };
        let _error = collect_selected_fixture_paths(&temp, &plan, vec![DriveLetter::L])
            .expect_err("undiscovered profiles should fail wildcard queries");
        Ok(())
    }
}
