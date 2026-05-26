use crate::query::IndexedPathRow;
use crate::query::QueryExecutionOptions;
use crate::query::QueryIgnoreBehavior;
use crate::query::QueryIgnoreRules;
use crate::query::QueryPlan;
use crate::query::matching_row_indices_for_rule;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "CLI flags map directly to independent query toggles"
)]
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
    /// Bypass the machine daemon and read published indexes directly
    #[facet(args::named, default)]
    pub no_daemon: bool,
    /// Allow falling back to published disk indexes if the daemon-only query path degrades
    #[facet(args::named, default)]
    pub allow_fallback: bool,
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

#[derive(Default, Facet, Arbitrary, Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
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
    let path = path.as_os_str().to_string_lossy().replace('/', "\\");
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map_or_else(|| path.clone(), |rest| format!(r"\\{rest}"));
    let path = path
        .strip_prefix(r"\\?\")
        .map_or_else(|| path.clone(), ToString::to_string);

    path.split('\\')
        .filter(|component| !component.is_empty())
        .map(str::to_ascii_lowercase)
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
        MappedSearchIndex::open(index_path).wrap_err_with(|| {
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

#[must_use]
fn spawn_streamed_query_row_drain(
    mut rows_rx: vox::Rx<teamy_mft_daemon_rpc::IndexedPathRowDto>,
) -> std::thread::JoinHandle<eyre::Result<Vec<IndexedPathRow>>> {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let mut response_rows = Vec::new();
            loop {
                match rows_rx.recv().await {
                    Ok(Some(row)) => response_rows.push(IndexedPathRow {
                        path: row.get().path.clone(),
                        has_deleted_entries: row.get().has_deleted_entries,
                        is_ignored: row.get().is_ignored,
                    }),
                    Ok(None) => break,
                    Err(error) => {
                        eyre::bail!("Failed receiving streamed daemon query rows: {error}")
                    }
                }
            }
            Ok(response_rows)
        })
    })
}

fn published_machine_drive_letters(cache_root: &Path, requested: Vec<char>) -> Vec<char> {
    requested
        .into_iter()
        .filter(|drive_letter| {
            let paths = crate::machine::config::published_drive_paths(cache_root, *drive_letter);
            paths.mft_path.is_file() && paths.base_index_path.is_file()
        })
        .collect()
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
    /// Returns an error if the query is empty, machine cache cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    pub fn invoke(self) -> eyre::Result<Vec<PathBuf>> {
        self.invoke_with_options(QueryExecutionOptions::default())
    }

    /// Run the query and return matching paths using explicit execution options.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, machine cache cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    pub fn invoke_with_options(self, options: QueryExecutionOptions) -> eyre::Result<Vec<PathBuf>> {
        self.collect_rows_with_options(options).map(|rows| {
            rows.into_iter()
                .map(|row| PathBuf::from(row.path))
                .collect()
        })
    }

    /// Run the query and print results to stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, machine cache cannot be retrieved,
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
    /// Returns an error if the query is empty, machine cache cannot be retrieved,
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

    /// # Errors
    ///
    /// Returns an error if the query is invalid, the daemon transport fails, or the published cache cannot be queried.
    #[allow(
        clippy::too_many_lines,
        reason = "This method centralizes the query source selection and fallback behavior"
    )]
    /// # Errors
    ///
    /// Returns an error if the query is empty, drive letters cannot be resolved,
    /// the machine cache is unavailable, the query scope cannot be canonicalized,
    /// or if daemon/disk-backed index reads fail.
    pub fn collect_rows_with_options(
        self,
        options: QueryExecutionOptions,
    ) -> eyre::Result<Vec<IndexedPathRow>> {
        debug!("Running query with args: {:?}", self);
        if self.query.iter().all(|query| query.trim().is_empty()) {
            eyre::bail!("query string required")
        }
        if self.no_daemon && matches!(options.source, QuerySource::DaemonOnly) {
            eyre::bail!("--no-daemon cannot be combined with daemon-only query mode");
        }
        let drive_letters = self.drive_letter_pattern.clone().into_drive_letters()?;
        let requested_source = if matches!(options.source, QuerySource::Auto) {
            self.source
        } else {
            options.source
        };
        let effective_source = if self.no_daemon {
            QuerySource::DiskOnly
        } else {
            requested_source
        };
        match effective_source {
            QuerySource::Auto => {
                let allow_auto_fallback = self.allow_fallback;
                let config = crate::machine::ipc::load_machine_daemon_client_config()?;
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
                match crate::machine::ipc::ensure_daemon_ready(&config) {
                    Ok(_ready_daemon) => {
                        let (rows_tx, rows_rx) =
                            vox::channel::<teamy_mft_daemon_rpc::IndexedPathRowDto>();
                        let (logs_tx, logs_rx) =
                            vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
                        let row_drain = spawn_streamed_query_row_drain(rows_rx);
                        let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
                        let query_outcome =
                            crate::machine::ipc::query_stream(&config, request, rows_tx, logs_tx);
                        let response_rows = row_drain.join().map_err(|join_error| {
                            eyre::eyre!("Daemon row drain thread panicked: {join_error:?}")
                        })??;
                        let _ = log_drain.join();
                        match query_outcome {
                            Ok(Ok(response)) => {
                                debug!(correlation_id = %response.correlation_id, "Daemon streamed query completed");
                                return Ok(response_rows);
                            }
                            Ok(Err(error)) => {
                                if matches!(
                                    error.kind,
                                    crate::machine::ipc::MachineErrorKind::RequestInvalid
                                ) {
                                    eyre::bail!(error.message);
                                }
                                if !allow_auto_fallback {
                                    return Err(format_daemon_query_error(&error));
                                }
                                tracing::warn!(kind = ?error.kind, error = %error.message, "Daemon query degraded; serving published machine cache");
                            }
                            Err(error) => {
                                if !allow_auto_fallback {
                                    return Err(error);
                                }
                                tracing::warn!(error = %error, "Daemon query failed; serving published machine cache");
                            }
                        }
                    }
                    Err(error) => {
                        if !allow_auto_fallback {
                            return Err(error);
                        }
                        tracing::warn!(error = %error, "Daemon readiness check failed; serving published machine cache");
                    }
                }
                let sync_dir = crate::machine::config::load_required_cache_root()?;
                let drive_letters = published_machine_drive_letters(&sync_dir, drive_letters);
                if drive_letters.is_empty() {
                    eyre::bail!(
                        "No machine-managed published drives matched the requested drive set"
                    );
                }
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
                let config = crate::machine::ipc::load_machine_daemon_client_config()?;
                crate::machine::ipc::ensure_daemon_ready(&config)?;
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
                let (rows_tx, rows_rx) = vox::channel::<teamy_mft_daemon_rpc::IndexedPathRowDto>();
                let (logs_tx, logs_rx) =
                    vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
                let row_drain = spawn_streamed_query_row_drain(rows_rx);
                let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
                let response =
                    crate::machine::ipc::query_stream(&config, request, rows_tx, logs_tx)?;
                let response_rows = row_drain.join().map_err(|join_error| {
                    eyre::eyre!("Daemon row drain thread panicked: {join_error:?}")
                })??;
                let _ = log_drain.join();
                match response {
                    Ok(response) => {
                        debug!(
                            correlation_id = %response.correlation_id,
                            "Daemon-only streamed query completed"
                        );
                        Ok(response_rows)
                    }
                    Err(error) => Err(format_daemon_query_error(&error)),
                }
            }
            QuerySource::DiskOnly => {
                let sync_dir = crate::machine::config::load_required_cache_root()?;
                let drive_letters = published_machine_drive_letters(&sync_dir, drive_letters);
                if drive_letters.is_empty() {
                    eyre::bail!(
                        "No machine-managed published drives matched the requested drive set"
                    );
                }
                self.collect_rows_from_sync_dir_and_drive_letters(&sync_dir, drive_letters, options)
            }
        }
    }

    /// # Errors
    ///
    /// Returns an error if the drive indexes cannot be loaded or the query scope cannot be applied.
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

fn format_daemon_query_error(error: &crate::machine::ipc::MachineError) -> eyre::Report {
    eyre::eyre!(
        "Daemon query failed ({:?}): {}. Re-run with `--allow-fallback` to query the published disk cache, or `--source disk-only`/`--no-daemon` to bypass live daemon state.",
        error.kind,
        error.message
    )
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
