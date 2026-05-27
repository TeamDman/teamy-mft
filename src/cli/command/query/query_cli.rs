use crate::presentation::ResultListPresentation;
use crate::query::DiskQueryExecutor;
use crate::query::IndexedPathRow;
use crate::query::QueryDataSource;
use crate::query::QueryIgnoreBehavior;
use crate::query::QueryPlan;
use crate::query::QueryRowStream;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::io::IsTerminal;
use std::path::Path;
use std::path::PathBuf;
use tracing::debug;
use tracing::instrument;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
#[facet(rename_all = "kebab-case")]
pub struct QueryArgs {
    #[facet(flatten)]
    pub plan: QueryPlan,
    /// Output density mode
    #[facet(args::named, default)]
    pub density: QueryDensity,
    /// Bypass the machine daemon and read published indexes directly
    #[facet(args::named, default)]
    pub no_daemon: bool,
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

impl QueryArgs {
    /// Create a new `QueryArgs` with the given query pattern and all other options at their defaults.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            plan: QueryPlan::new(pattern),
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

    fn data_source(&self) -> QueryDataSource {
        if self.no_daemon {
            QueryDataSource::DiskOnly
        } else {
            QueryDataSource::DaemonOnly
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
        self.collect_rows().map(|rows| {
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
    #[instrument(level = "info", skip_all, fields(query = ?self.plan.query, query_scope = ?self.plan.r#in, limit = self.plan.limit, include_deleted = self.plan.include_deleted, only_deleted = self.plan.only_deleted, show_ignored = self.plan.show_ignored, only_ignored = self.plan.only_ignored, density = ?self.density))]
    pub fn invoke_and_print(self) -> eyre::Result<()> {
        let limit = self.plan.limit;
        let density = self.density;
        let include_deleted = self.plan.include_deleted;
        let only_deleted = self.plan.only_deleted;
        let show_ignored = self.plan.show_ignored;
        let only_ignored = self.plan.only_ignored;

        let results = self.collect_rows()?;

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
    /// Returns an error if the query is empty or invalid, drive letters cannot be resolved,
    /// the daemon transport fails, the machine cache is unavailable, the query scope cannot
    /// be canonicalized, or if daemon/disk-backed index reads fail.
    #[allow(
        clippy::too_many_lines,
        reason = "This method centralizes the query source selection behavior"
    )]
    pub fn collect_rows(self) -> eyre::Result<Vec<IndexedPathRow>> {
        debug!("Running query with args: {:?}", self);
        let query_inputs = self.plan.query_inputs();
        if query_inputs.is_empty() {
            eyre::bail!("query string required");
        }
        for (index, query) in query_inputs.iter().enumerate() {
            if query.is_empty() {
                eyre::bail!("query argument {index} is empty; pass a non-empty query string");
            }
            if query.trim().is_empty() {
                // Preserve whitespace-only queries because whitespace can be an
                // intentional path-name search. Warn because it is commonly accidental.
                tracing::warn!(
                    query_index = index,
                    query = ?query,
                    "Query argument contains only whitespace"
                );
            }
        }
        let data_source = self.data_source();
        let request = self.plan.query_request(
            self.plan
                .drive_letter_pattern
                .clone()
                .into_drive_letters()?,
        );
        let limit = request.limit;

        match data_source {
            QueryDataSource::DaemonOnly => {
                let config = crate::machine::ipc::load_machine_daemon_client_config()?;
                crate::machine::ipc::ensure_daemon_ready(&config)?;
                let (rows_tx, rows_rx) = vox::channel::<teamy_mft_daemon_rpc::IndexedPathRowDto>();
                let (logs_tx, logs_rx) =
                    vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
                let row_drain = spawn_query_row_drain(QueryRowStream::Vox(rows_rx), limit);
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
            QueryDataSource::DiskOnly => {
                let executor = DiskQueryExecutor::new(
                    &crate::machine::config::load_required_cache_root()?,
                    request,
                    QueryIgnoreBehavior::AutoDiscover,
                )?;
                drain_query_stream(executor.stream()?, limit)
            }
        }
    }
}

fn format_daemon_query_error(error: &crate::machine::ipc::MachineError) -> eyre::Report {
    eyre::eyre!(
        "Daemon query failed ({:?}): {}. Re-run with `--no-daemon` to query the published disk cache.",
        error.kind,
        error.message
    )
}

#[cfg(test)]
mod tests {
    use crate::query::QueryPlan as ParsedQueryPlan;
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
        let plan = ParsedQueryPlan::parse_inputs(&[String::from("flow .jar$")])?;

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
