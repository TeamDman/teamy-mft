use crate::presentation::ResultListPresentation;
use crate::query::DiskQueryExecutor;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryRowStream;
use arbitrary::Arbitrary;
use eyre::Context;
use eyre::ensure;
use facet::Facet;
use figue::{self as args};
use std::io::IsTerminal;
use tracing::debug;
use tracing::instrument;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
#[facet(rename_all = "kebab-case")]
pub struct QueryArgs {
    #[facet(flatten)]
    pub plan: QueryPlan,
    /// Output density mode
    #[facet(args::named, default)]
    pub density: QueryResultsOutputDensity,
    /// Bypass the machine daemon and read published indexes directly
    #[facet(args::named, default)]
    pub no_daemon: bool,
    /// Ask the machine daemon to run the query
    #[facet(args::named, default)]
    pub daemon: bool,
}

#[derive(Default, Facet, Arbitrary, Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum QueryResultsOutputDensity {
    #[default]
    Auto,
    Lines,
    Columns,
}

impl QueryArgs {
    /// Create a new `QueryArgs` with the given query pattern and all other options at their defaults.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            plan: QueryPlan::new(pattern),
            ..Default::default()
        }
    }

    /// Run the query and print results to stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, machine cache cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    #[instrument(level = "info", skip_all, fields(query = ?self.plan.query, query_scope = ?self.plan.r#in, profile = ?self.plan.profile, limit = ?self.plan.limit, include_deleted = self.plan.include_deleted, only_deleted = self.plan.only_deleted, show_ignored = self.plan.show_ignored, only_ignored = self.plan.only_ignored, density = ?self.density))]
    pub fn invoke_and_print(self) -> eyre::Result<()> {
        let results = self.collect_rows()?;

        let stdout_is_terminal = std::io::stdout().is_terminal();
        let colorize = stdout_is_terminal
            && (self.plan.include_deleted
                || self.plan.only_deleted
                || self.plan.show_ignored
                || self.plan.only_ignored);
        let result_limit = self
            .plan
            .limit
            .map(std::convert::Into::into)
            .unwrap_or(results.len())
            .min(results.len());
        let display_results = &results[..result_limit];
        let presentation = ResultListPresentation::for_terminal();
        let mut stdout = std::io::stdout().lock();
        let use_columns = match self.density {
            QueryResultsOutputDensity::Auto => stdout_is_terminal,
            QueryResultsOutputDensity::Lines => false,
            QueryResultsOutputDensity::Columns => true,
        };
        presentation.write_result_list(
            display_results,
            &mut stdout,
            use_columns,
            |row| row.path.as_str().chars().count(),
            |row, writer| row.render_path(writer, colorize),
        )?;

        Ok(())
    }

    /// Emit warnings for any potentially unintentional query patterns and return an error if the query is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty
    pub fn check_query(&self) -> eyre::Result<()> {
        if self.plan.query.is_empty() {
            eyre::bail!("query must not be empty");
        }
        for (index, group) in self.plan.query.groups().iter().enumerate() {
            if group.is_empty() {
                eyre::bail!("query argument {index} is empty; pass a non-empty query string");
            }
            for (rule_index, rule) in group.rules.iter().enumerate() {
                if rule.is_empty() {
                    // Preserve whitespace-only queries because whitespace can be an
                    // intentional path-name search. Warn because it is commonly accidental.
                    tracing::warn!(
                        query_index = index,
                        rule_index = rule_index,
                        query = ?group,
                        "Query rule contains only whitespace"
                    );
                }
            }
        }
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
    pub fn collect_rows(&self) -> eyre::Result<Vec<QueryResultRow>> {
        debug!("Running query with args: {:?}", self);
        self.check_query()?;
        self.plan.ensure_selected_profile_allowed()?;
        ensure!(
            !(self.daemon && self.no_daemon),
            "`--daemon` and `--no-daemon` cannot be used together"
        );

        let rtn = if self.daemon {
            let _ctrl_c_guard = crate::windows_utils::ctrl_c::use_graceful_cancellation();
            let config = crate::machine::ipc::load_machine_daemon_client_config()?;
            crate::machine::ipc::ensure_daemon_ready(&config)?;
            let (rows_tx, rows_rx) = vox::channel::<QueryResultRow>();
            let (logs_tx, logs_rx) =
                vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
            let (cancel_tx, cancel_rx) = vox::channel::<u8>();
            let row_drain = {
                let stream = QueryRowStream::Vox(rows_rx);
                let limit = self.plan.limit;
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(stream.collect_filtered_limit(limit))
                })
            };
            let _cancel_signal = std::thread::spawn(move || {
                while !crate::windows_utils::ctrl_c::interrupted() {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async move {
                    let _ = cancel_tx.send(1).await;
                    let _ = cancel_tx.close(Vec::new()).await;
                });
                Ok::<(), eyre::Report>(())
            });
            let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
            let response = crate::machine::ipc::query_stream(
                &config,
                self.plan.clone(),
                rows_tx,
                logs_tx,
                cancel_rx,
            )
            .wrap_err(
                "Daemon query failed, re-run without `--daemon` to query the published disk cache",
            )?;
            let response_rows = row_drain.join().map_err(|join_error| {
                eyre::eyre!("Daemon row drain thread panicked: {join_error:?}")
            })??;
            let () = log_drain.join().map_err(|join_error| {
                eyre::eyre!("Daemon log drain thread panicked: {join_error:?}")
            })?;
            debug!(
                correlation_id = %response,
                "Daemon-only streamed query completed"
            );
            response_rows
        } else {
            let executor = DiskQueryExecutor::new(self.plan.clone())?;
            let stream = executor.stream()?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(stream.collect_filtered_limit(self.plan.limit))?
        };
        if let Some(limit) = **self.plan.limit {
            ensure!(
                rtn.len() <= limit.into(),
                "Collected more results ({}) than the specified limit ({})",
                rtn.len(),
                limit
            );
        }
        Ok(rtn)
    }
}
