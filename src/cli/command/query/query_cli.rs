use crate::query::IndexedPathRow;
use crate::query::QueryExecutionOptions;
use crate::query::QueryIgnoreBehavior;
use crate::query::QueryIgnoreRules;
use crate::query::QueryPlan;
use crate::query::matching_row_indices_for_rule;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use crate::sync_dir::SYNC_DIR_ENV;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use color_eyre::owo_colors::OwoColorize;
use eyre::Context;
use facet::Facet;
use figue::{self as args};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::Path;
use std::path::PathBuf;
use teamy_windows::storage::DriveLetterPattern;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use tracing::instrument;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
#[facet(rename_all = "kebab-case")]
// cli[impl command.query.drive-pattern-selection]
pub struct QueryArgs {
    /// Fast query groups. Each positional argument is `OR`ed; whitespace-delimited terms within one argument are `AND`ed.
    #[facet(args::positional, default)]
    pub query: Vec<String>,
    /// Restrict results to this path. Directories include descendants; files match exactly.
    #[facet(args::named, default)]
    pub r#in: Option<String>,
    /// Drive letter pattern to match drives whose cached MFTs will be queried (e.g., "*", "C", "CD", "C,D")
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Maximum number of results to show
    #[facet(args::named, default)]
    pub limit: usize,
    /// Include paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub include_deleted: bool,
    /// Show only paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub only_deleted: bool,
    /// Include paths hidden by `.teamymftignore` rules
    #[facet(args::named, default)]
    pub show_ignored: bool,
    /// Show only paths hidden by `.teamymftignore` rules
    #[facet(args::named, default)]
    pub only_ignored: bool,
    /// Output density mode
    #[facet(args::named, default)]
    pub density: QueryDensity,
    /// Query source selection
    #[facet(args::named, default)]
    pub source: QuerySource,
}

#[derive(Default, Facet, Arbitrary, Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum QueryDensity {
    #[default]
    Auto,
    Lines,
    Columns,
}

#[derive(
    Default,
    Facet,
    Arbitrary,
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum QuerySource {
    #[default]
    Auto,
    DaemonOnly,
    DiskOnly,
}

#[derive(Debug, Default)]
struct DriveQueryResult {
    loaded_rows: usize,
    matched_rows: Vec<IndexedPathRow>,
}

#[derive(Debug, Clone)]
struct QueryScope {
    root: PathBuf,
    include_descendants: bool,
}

fn render_indexed_path(row: &IndexedPathRow, colorize: bool) -> String {
    if !colorize {
        return row.path.clone();
    }
    if row.is_ignored {
        return row.path.yellow().to_string();
    }
    if row.has_deleted_entries {
        row.path.red().to_string()
    } else {
        row.path.green().to_string()
    }
}

fn string_display_width(value: &str) -> usize {
    value.chars().count()
}

fn detect_terminal_columns() -> Option<usize> {
    crossterm::terminal::size()
        .ok()
        .map(|(columns, _)| usize::from(columns))
        .filter(|value| *value > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
}

fn print_results_lines(results: &[IndexedPathRow], colorize: bool) {
    for row in results {
        println!("{}", render_indexed_path(row, colorize));
    }
}

fn print_results_columns(results: &[IndexedPathRow], colorize: bool) {
    if results.is_empty() {
        return;
    }

    let gap = 2usize;
    let max_width = results
        .iter()
        .map(|row| string_display_width(&row.path))
        .max()
        .unwrap_or(1)
        .max(1);
    let terminal_columns = detect_terminal_columns().unwrap_or(120usize);

    let column_count = ((terminal_columns + gap) / (max_width + gap)).max(1);
    let row_count = results.len().div_ceil(column_count);

    for row_index in 0..row_count {
        let mut line = String::new();

        for column_index in 0..column_count {
            let index = row_index + column_index * row_count;
            if index >= results.len() {
                continue;
            }

            let row = &results[index];
            line.push_str(&render_indexed_path(row, colorize));

            if column_index + 1 < column_count {
                let pad = (max_width + gap).saturating_sub(string_display_width(&row.path));
                line.push_str(&" ".repeat(pad));
            }
        }

        println!("{line}");
    }
}

fn should_include_indexed_row(
    include_deleted: bool,
    only_deleted: bool,
    has_deleted_entries: bool,
) -> bool {
    // cli[impl command.query.deleted-filter]
    if only_deleted {
        return has_deleted_entries;
    }

    include_deleted || !has_deleted_entries
}

fn should_include_ignored_row(show_ignored: bool, only_ignored: bool, is_ignored: bool) -> bool {
    if only_ignored {
        return is_ignored;
    }

    show_ignored || !is_ignored
}

fn resolve_query_scope(scope: Option<&str>) -> eyre::Result<Option<QueryScope>> {
    let Some(scope) = scope else {
        return Ok(None);
    };

    let root = dunce::canonicalize(scope)
        .wrap_err_with(|| format!("Failed resolving query scope from {scope}"))?;

    Ok(Some(QueryScope {
        include_descendants: root.is_dir(),
        root,
    }))
}

fn lowercase_path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect()
}

fn path_matches_scope(path: &Path, scope: &QueryScope) -> bool {
    if cfg!(windows) {
        let path_components = lowercase_path_components(path);
        let scope_components = lowercase_path_components(&scope.root);

        return if scope.include_descendants {
            path_components.starts_with(&scope_components)
        } else {
            path_components == scope_components
        };
    }

    if scope.include_descendants {
        path.starts_with(&scope.root)
    } else {
        path == scope.root
    }
}

fn should_include_scope(path: &str, scope: Option<&QueryScope>) -> bool {
    // cli[impl command.query.scope-filter]
    let Some(scope) = scope else {
        return true;
    };

    path_matches_scope(Path::new(path), scope)
}

fn load_and_query_search_index(
    index_path: &Path,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<DriveQueryResult> {
    let _span = info_span!("load_drive_search_index").entered();
    {
        let _span = info_span!("validate_search_index_file").entered();
        if !index_path.is_file() {
            eyre::bail!("Fast query requires {}.", index_path.display(),);
        }
    }

    let mapped = {
        let _span = info_span!("map_search_index_file").entered();
        MappedSearchIndex::open(&index_path).wrap_err_with(|| {
            format!("Failed loading search index from {}", index_path.display())
        })?
    };

    let parsed_index = {
        let _span = info_span!("parse_search_index_for_query").entered();
        SearchIndexBytes::new(mapped.bytes())
            .parse_trusted_for_query()
            .wrap_err_with(|| {
                format!(
                    "Failed preparing search index rows from {}",
                    index_path.display()
                )
            })?
    };

    let loaded_rows = parsed_index.row_count();
    let matched_row_indices = {
        let _span = info_span!("match_search_index_postings").entered();
        query_plan
            .matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed_index, rule))
            .wrap_err_with(|| {
                format!(
                    "Failed matching search index rows from {}",
                    index_path.display()
                )
            })?
    };
    let matched_rows = {
        let _span = info_span!("materialize_matched_index_rows").entered();
        let mut matched_rows = Vec::with_capacity(matched_row_indices.len());

        for row_index in matched_row_indices {
            let row = parsed_index
                .row_view(row_index as usize)
                .wrap_err_with(|| {
                    format!(
                        "Failed materializing search index row {} from {}",
                        row_index,
                        index_path.display()
                    )
                })?;

            if !should_include_indexed_row(include_deleted, only_deleted, row.has_deleted_entries) {
                continue;
            }

            matched_rows.push(IndexedPathRow {
                path: row.path(),
                has_deleted_entries: row.has_deleted_entries,
                is_ignored: false,
            });
        }

        matched_rows
    };

    Ok(DriveQueryResult {
        loaded_rows,
        matched_rows,
    })
}

fn load_and_query_drive_search_index(
    drive_letter: char,
    sync_dir: &Path,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<DriveQueryResult> {
    let base_index_path = sync_dir.join(format!("{drive_letter}.mft_search_index"));
    let overlay_index_path = sync_dir.join(format!("{drive_letter}.mft_overlay_search_index"));
    let mut result =
        load_and_query_search_index(&base_index_path, query_plan, include_deleted, only_deleted)
            .wrap_err_with(|| {
                format!(
                    "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
                    base_index_path.display(),
                    drive_letter
                )
            })?;

    if overlay_index_path.is_file() {
        let overlay_result = load_and_query_search_index(
            &overlay_index_path,
            query_plan,
            include_deleted,
            only_deleted,
        )?;
        result.loaded_rows += overlay_result.loaded_rows;
        result.matched_rows = merge_rows(result.matched_rows, overlay_result.matched_rows);
    }

    Ok(result)
}

fn merge_rows(
    base_rows: Vec<IndexedPathRow>,
    overlay_rows: Vec<IndexedPathRow>,
) -> Vec<IndexedPathRow> {
    let mut merged = BTreeMap::<String, IndexedPathRow>::new();
    for row in base_rows {
        merged.insert(row.path.clone(), row);
    }
    for row in overlay_rows {
        merged.insert(row.path.clone(), row);
    }
    merged.into_values().collect()
}

impl QueryArgs {
    /// Create a new `QueryArgs` with the given query pattern and all other options at their defaults.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            query: vec![pattern.into()],
            ..Default::default()
        }
    }

    fn use_columns(density: QueryDensity, stdout_is_terminal: bool) -> bool {
        match density {
            QueryDensity::Auto => stdout_is_terminal,
            QueryDensity::Lines => false,
            QueryDensity::Columns => true,
        }
    }

    /// Run the query and return matching paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    pub fn invoke(self) -> eyre::Result<Vec<PathBuf>> {
        let rows = self.collect_rows_with_options(QueryExecutionOptions::default())?;
        Ok(rows.into_iter().map(|r| PathBuf::from(r.path)).collect())
    }

    /// Run the query and return matching paths using explicit execution options.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    pub fn invoke_with_options(self, options: QueryExecutionOptions) -> eyre::Result<Vec<PathBuf>> {
        let rows = self.collect_rows_with_options(options)?;
        Ok(rows.into_iter().map(|r| PathBuf::from(r.path)).collect())
    }

    /// Run the query and print results to stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    #[instrument(level = "info", skip_all, fields(query = ?self.query, query_scope = ?self.r#in, limit = self.limit, include_deleted = self.include_deleted, only_deleted = self.only_deleted, show_ignored = self.show_ignored, only_ignored = self.only_ignored, density = ?self.density))]
    pub fn invoke_and_print(self) -> eyre::Result<()> {
        self.invoke_and_print_with_options(QueryExecutionOptions::default())
    }

    /// Run the query and print results to stdout using explicit execution options.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    pub fn invoke_and_print_with_options(self, options: QueryExecutionOptions) -> eyre::Result<()> {
        let limit = self.limit;
        let density = self.density;
        let include_deleted = self.include_deleted;
        let only_deleted = self.only_deleted;
        let show_ignored = self.show_ignored;
        let only_ignored = self.only_ignored;

        let results = self.collect_rows_with_options(options)?;

        let stdout_is_terminal = std::io::stdout().is_terminal();
        let colorize =
            stdout_is_terminal && (include_deleted || only_deleted || show_ignored || only_ignored);
        let result_limit = if limit == 0 {
            results.len()
        } else {
            limit.min(results.len())
        };
        let display_results = &results[..result_limit];

        if Self::use_columns(density, stdout_is_terminal) {
            print_results_columns(display_results, colorize);
        } else {
            print_results_lines(display_results, colorize);
        }

        Ok(())
    }

    pub fn collect_rows_with_options(
        self,
        options: QueryExecutionOptions,
    ) -> eyre::Result<Vec<IndexedPathRow>> {
        debug!("Running query with args: {:?}", self);
        if self.query.iter().all(|query| query.trim().is_empty()) {
            eyre::bail!("query string required")
        }
        let drive_letters = self.drive_letter_pattern.clone().into_drive_letters()?;
        match options.source {
            QuerySource::Auto => {
                if let Some(sync_dir) = sync_dir_from_env() {
                    return self.collect_rows_from_sync_dir_and_drive_letters(
                        &sync_dir,
                        drive_letters,
                        QueryExecutionOptions {
                            ignore: options.ignore,
                            source: QuerySource::DiskOnly,
                        },
                    );
                }

                if let Some(config) = crate::machine::config::load_machine_config()? {
                    if !matches!(
                        crate::machine::service::query_service_state(&config.service_name)?,
                        crate::machine::service::WindowsServiceState::Missing
                    ) {
                        match crate::machine::service::start_service_if_needed(&config.service_name)
                        {
                            Ok(()) => {
                                let request = crate::machine::ipc::QueryRequest {
                                    query: self.query.clone(),
                                    query_scope: self.r#in.clone(),
                                    drive_letters: drive_letters.clone(),
                                    limit: self.limit,
                                    include_deleted: self.include_deleted,
                                    only_deleted: self.only_deleted,
                                    show_ignored: self.show_ignored,
                                    only_ignored: self.only_ignored,
                                };
                                match crate::machine::ipc::send_request(
                                    &config,
                                    &crate::machine::ipc::MachineRequest::Query(request),
                                ) {
                                    Ok(crate::machine::ipc::MachineResponse::Query(response)) => {
                                        return Ok(response.rows);
                                    }
                                    Ok(crate::machine::ipc::MachineResponse::Error(error)) => {
                                        match error.kind {
                                            crate::machine::ipc::MachineErrorKind::RequestInvalid => {
                                                eyre::bail!(error.message)
                                            }
                                            crate::machine::ipc::MachineErrorKind::Unavailable
                                            | crate::machine::ipc::MachineErrorKind::Degraded => {
                                                tracing::warn!(
                                                    kind = ?error.kind,
                                                    error = %error.message,
                                                    "Daemon query reported degraded state, falling back to disk"
                                                );
                                            }
                                        }
                                    }
                                    Ok(other) => {
                                        tracing::warn!(?other, "Unexpected daemon response, falling back to disk");
                                    }
                                    Err(error) => {
                                        tracing::warn!(error = %error, "Daemon query failed, falling back to disk");
                                    }
                                }
                            }
                            Err(error) => {
                                tracing::warn!(error = %error, "Daemon start failed, falling back to disk");
                            }
                        }
                    }

                    return self.collect_rows_from_sync_dir_and_drive_letters(
                        &config.cache_root,
                        drive_letters,
                        QueryExecutionOptions {
                            ignore: options.ignore,
                            source: QuerySource::DiskOnly,
                        },
                    );
                }

                let sync_dir = {
                    let _span = info_span!("resolve_sync_dir").entered();
                    try_get_sync_dir()?
                };
                self.collect_rows_from_sync_dir_and_drive_letters(
                    &sync_dir,
                    drive_letters,
                    QueryExecutionOptions {
                        ignore: options.ignore,
                        source: QuerySource::DiskOnly,
                    },
                )
            }
            QuerySource::DaemonOnly => {
                let config = crate::machine::config::load_machine_config()?.ok_or_else(|| {
                    eyre::eyre!("Machine daemon is not installed. Run `teamy-mft install` first.")
                })?;
                crate::machine::service::start_service_if_needed(&config.service_name)?;
                let request = crate::machine::ipc::QueryRequest {
                    query: self.query,
                    query_scope: self.r#in,
                    drive_letters,
                    limit: self.limit,
                    include_deleted: self.include_deleted,
                    only_deleted: self.only_deleted,
                    show_ignored: self.show_ignored,
                    only_ignored: self.only_ignored,
                };
                match crate::machine::ipc::send_request(
                    &config,
                    &crate::machine::ipc::MachineRequest::Query(request),
                )? {
                    crate::machine::ipc::MachineResponse::Query(response) => Ok(response.rows),
                    crate::machine::ipc::MachineResponse::Error(error) => {
                        eyre::bail!(error.message)
                    }
                    other => eyre::bail!("Unexpected daemon response: {:?}", other),
                }
            }
            QuerySource::DiskOnly => {
                let sync_dir = if let Some(sync_dir) = sync_dir_from_env() {
                    sync_dir
                } else if let Some(config) = crate::machine::config::load_machine_config()? {
                    config.cache_root
                } else {
                    try_get_sync_dir()?
                };
                self.collect_rows_from_sync_dir_and_drive_letters(&sync_dir, drive_letters, options)
            }
        }
    }

    pub fn collect_rows_from_sync_dir_and_drive_letters(
        self,
        sync_dir: &Path,
        drive_letters: Vec<char>,
        options: QueryExecutionOptions,
    ) -> eyre::Result<Vec<IndexedPathRow>> {
        let mft_files: Vec<(char, PathBuf)> = {
            let _span = info_span!("discover_mft_files").entered();
            drive_letters
                .into_iter()
                .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
                .filter(|(_, p)| p.is_file())
                .collect()
        };
        self.collect_rows_from_files(mft_files, sync_dir, options)
    }

    fn collect_rows_from_files(
        self,
        mft_files: Vec<(char, PathBuf)>,
        sync_dir: &Path,
        options: QueryExecutionOptions,
    ) -> eyre::Result<Vec<IndexedPathRow>> {
        let query_plan = {
            let _span = info_span!("parse_query_rules", query = ?self.query).entered();
            QueryPlan::parse_inputs(&self.query)?
        };
        let query_scope = {
            let _span = info_span!("resolve_query_scope", query_scope = ?self.r#in).entered();
            resolve_query_scope(self.r#in.as_deref())?
        };
        let drive_letters = mft_files
            .iter()
            .map(|(drive_letter, _)| *drive_letter)
            .collect::<Vec<_>>();
        let ignore_rules = match options.ignore {
            QueryIgnoreBehavior::AutoDiscover => Some(
                QueryIgnoreRules::discover_for_drive_letters(&drive_letters, sync_dir)?,
            ),
            QueryIgnoreBehavior::Disabled => None,
            QueryIgnoreBehavior::Custom(rules) => Some(rules),
        };

        let mut loaded_rows = 0usize;
        let mut results = Vec::new();
        {
            let _span = info_span!("load_search_indexes", drives = mft_files.len()).entered();
            let include_deleted = self.include_deleted;
            let only_deleted = self.only_deleted;
            let show_ignored = self.show_ignored;
            let only_ignored = self.only_ignored;
            let load_results: Vec<eyre::Result<DriveQueryResult>> = mft_files
                .into_par_iter()
                .map(|(drive_letter, _)| {
                    load_and_query_drive_search_index(
                        drive_letter,
                        sync_dir,
                        &query_plan,
                        include_deleted,
                        only_deleted,
                    )
                })
                .collect();

            for result in load_results {
                let result = result?;
                loaded_rows += result.loaded_rows;
                results.extend(result.matched_rows.into_iter().filter_map(|mut row| {
                    if !should_include_scope(&row.path, query_scope.as_ref()) {
                        return None;
                    }

                    row.is_ignored = ignore_rules
                        .as_ref()
                        .is_some_and(|rules| rules.is_ignored_path(Path::new(&row.path)));

                    should_include_ignored_row(show_ignored, only_ignored, row.is_ignored)
                        .then_some(row)
                }));
            }
        }

        info!(
            loaded_rows = loaded_rows,
            matched = results.len(),
            total = loaded_rows,
            "Indexed query completed"
        );

        Ok(results)
    }
}

fn sync_dir_from_env() -> Option<PathBuf> {
    std::env::var(SYNC_DIR_ENV).ok().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_query_scope;
    use super::should_include_ignored_row;
    use super::should_include_scope;
    use crate::query::QueryPlan;
    use crate::query::QueryRule;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::OnceLock;

    fn current_dir_lock() -> &'static Mutex<()> {
        static CURRENT_DIR_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        CURRENT_DIR_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct CurrentDirRestore(PathBuf);

    impl Drop for CurrentDirRestore {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    fn parse_fixture_index()
    -> eyre::Result<crate::search_index::search_index_bytes::ParsedSearchIndex<'static>> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\src\\flower.jar"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\flowchart.txt"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\trees.zip"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        SearchIndexBytes::new(bytes).parse_trusted_for_query()
    }

    #[test]
    fn contains_rules_return_rows_from_trigram_candidates() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = QueryRule::parse("ower").expect("rule should parse");

        assert_eq!(
            crate::query::matching_row_indices_for_rule(&parsed, &rule)?,
            vec![0]
        );

        Ok(())
    }

    #[test]
    fn short_contains_rules_still_match_without_trigrams() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = QueryRule::parse("fl").expect("rule should parse");

        assert_eq!(
            crate::query::matching_row_indices_for_rule(&parsed, &rule)?,
            vec![0, 1]
        );

        Ok(())
    }

    #[test]
    fn query_plan_intersects_contains_and_suffix_candidates() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let plan = QueryPlan::parse_inputs(&[String::from("flow .jar$")])?;

        assert_eq!(
            plan.matching_row_indices(&|rule| crate::query::matching_row_indices_for_rule(
                &parsed, rule
            ))?,
            vec![0]
        );

        Ok(())
    }

    #[test]
    fn suffix_rules_match_only_terminal_segments_in_indexed_queries() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\repo\\project.git"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\.git\\objects\\pack\\pack-a.rev"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\.git\\refs\\remotes\\origin\\main"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        let parsed = SearchIndexBytes::new(bytes).parse_trusted_for_query()?;
        let rule = QueryRule::parse(".git$").expect("rule should parse");

        assert_eq!(
            crate::query::matching_row_indices_for_rule(&parsed, &rule)?,
            vec![0]
        );

        Ok(())
    }

    #[test]
    fn query_scope_directory_matches_descendants_but_not_sibling_prefixes() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_dir = temp_dir.path().join("repo");
        let nested_file = scope_dir.join("music").join("song.mp3");
        let sibling_file = temp_dir.path().join("repo2").join("song.mp3");

        std::fs::create_dir_all(
            nested_file
                .parent()
                .expect("nested file should have parent"),
        )?;
        std::fs::create_dir_all(
            sibling_file
                .parent()
                .expect("sibling file should have parent"),
        )?;
        std::fs::write(&nested_file, [])?;
        std::fs::write(&sibling_file, [])?;

        let scope = resolve_query_scope(Some(&scope_dir.to_string_lossy()))?
            .expect("directory scope should resolve");

        assert!(should_include_scope(
            &nested_file.to_string_lossy(),
            Some(&scope)
        ));
        assert!(!should_include_scope(
            &sibling_file.to_string_lossy(),
            Some(&scope)
        ));

        Ok(())
    }

    #[test]
    fn query_scope_file_matches_only_exact_path() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_file = temp_dir.path().join("track.flac");
        let other_file = temp_dir.path().join("track.flac.bak");

        std::fs::write(&scope_file, [])?;
        std::fs::write(&other_file, [])?;

        let scope = resolve_query_scope(Some(&scope_file.to_string_lossy()))?
            .expect("file scope should resolve");

        assert!(should_include_scope(
            &scope_file.to_string_lossy(),
            Some(&scope)
        ));
        assert!(!should_include_scope(
            &other_file.to_string_lossy(),
            Some(&scope)
        ));

        Ok(())
    }

    #[test]
    fn query_scope_dot_resolves_against_current_working_directory() -> eyre::Result<()> {
        let _lock = current_dir_lock()
            .lock()
            .expect("current dir test lock should not be poisoned");
        let temp_dir = tempfile::tempdir()?;
        let original_dir = std::env::current_dir()?;
        let _restore = CurrentDirRestore(original_dir);

        std::env::set_current_dir(temp_dir.path())?;

        let scope = resolve_query_scope(Some("."))?.expect("dot scope should resolve");

        assert_eq!(scope.root, dunce::canonicalize(temp_dir.path())?);
        assert!(scope.include_descendants);

        Ok(())
    }

    #[test]
    fn ignored_rows_are_hidden_by_default() {
        assert!(!should_include_ignored_row(false, false, true));
        assert!(should_include_ignored_row(false, false, false));
    }

    #[test]
    fn show_ignored_includes_both_visible_and_ignored_rows() {
        assert!(should_include_ignored_row(true, false, true));
        assert!(should_include_ignored_row(true, false, false));
    }

    #[test]
    fn only_ignored_filters_to_ignored_rows() {
        assert!(should_include_ignored_row(false, true, true));
        assert!(!should_include_ignored_row(true, true, false));
    }
}
