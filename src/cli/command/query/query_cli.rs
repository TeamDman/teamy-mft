use crate::presentation::ResultListPresentation;
use crate::query::DiskQueryExecutor;
use crate::query::IndexedPathRow;
use crate::query::QueryExecutionOptions;
use crate::query::QueryRequestSpec;
use crate::query::QueryRowStream;
use crate::query::QuerySource;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::io::IsTerminal;
use std::path::Path;
use std::path::PathBuf;
use teamy_windows::storage::DriveLetterPattern;
use tracing::debug;
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

#[must_use]
fn spawn_query_row_drain(
    stream: QueryRowStream,
    limit: usize,
) -> std::thread::JoinHandle<eyre::Result<Vec<IndexedPathRow>>> {
    std::thread::spawn(move || drain_query_stream(stream, limit))
}

fn drain_query_stream(stream: QueryRowStream, limit: usize) -> eyre::Result<Vec<IndexedPathRow>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(stream.collect_filtered_limit(limit))
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

    fn request_spec(&self, drive_letters: Vec<char>, source: QuerySource) -> QueryRequestSpec {
        QueryRequestSpec {
            query: self.query.clone(),
            query_scope: self.r#in.clone(),
            drive_letters,
            limit: self.limit,
            include_deleted: self.include_deleted,
            only_deleted: self.only_deleted,
            show_ignored: self.show_ignored,
            only_ignored: self.only_ignored,
            source,
            allow_fallback: self.allow_fallback,
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
        let presentation = ResultListPresentation::for_terminal();
        let mut stdout = std::io::stdout().lock();
        presentation.write_result_list(
            display_results,
            &mut stdout,
            Self::use_columns(density, stdout_is_terminal),
            |row| row.path.as_str().chars().count(),
            |row, writer| row.render_path(writer, colorize),
        )?;

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
                        let row_drain =
                            spawn_query_row_drain(QueryRowStream::Vox(rows_rx), self.limit);
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
                let row_drain = spawn_query_row_drain(QueryRowStream::Vox(rows_rx), self.limit);
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
        let spec = self.request_spec(drive_letters.clone(), QuerySource::DiskOnly);
        let executor = DiskQueryExecutor::new(sync_dir, drive_letters, spec, options.ignore);
        drain_query_stream(executor.stream()?, self.limit)
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
    use crate::query::QueryPlan;
    use crate::query::QueryRule;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;

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
}
