use crate::presentation::ResultListPresentation;
use crate::query::PreparedQueryStream;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryRuntime;
use arbitrary::Arbitrary;
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
    #[instrument(level = "info", skip_all, fields(query = ?self.plan.query, query_scope = ?self.plan.r#in, profile = ?self.plan.profile, limit = ?self.plan.limit, include_deleted = self.plan.include_deleted, only_deleted = self.plan.only_deleted, show_filtered = self.plan.show_filtered, only_filtered = self.plan.only_filtered, density = ?self.density))]
    pub fn invoke_and_print(self) -> eyre::Result<()> {
        let results = self.collect_rows()?;

        let stdout_is_terminal = std::io::stdout().is_terminal();
        let colorize = stdout_is_terminal
            && (self.plan.include_deleted
                || self.plan.only_deleted
                || self.plan.show_filtered
                || self.plan.only_filtered);
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
    pub fn collect_rows(&self) -> eyre::Result<Vec<QueryResultRow>> {
        debug!("Running query with args: {:?}", self);
        self.check_query()?;
        ensure!(
            !(self.daemon && self.no_daemon),
            "`--daemon` and `--no-daemon` cannot be used together"
        );
        self.plan.ensure_selected_profile_allowed()?;

        let rtn = self.runtime().collect_rows(self.plan.clone())?;
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

    pub fn runtime(&self) -> QueryRuntime {
        if self.daemon {
            QueryRuntime::daemon_rpc()
        } else {
            QueryRuntime::published_index_only()
        }
    }

    pub fn stream(&self) -> eyre::Result<PreparedQueryStream> {
        self.runtime().prepare_stream(self.plan.clone())
    }

    
}

#[cfg(test)]
mod tests {
    use super::QueryArgs;
    use crate::query::QueryRuntime;

    #[test]
    fn default_and_no_daemon_query_args_use_published_index_runtime() {
        let default_args = QueryArgs::new("Cargo.toml");
        let no_daemon_args = QueryArgs {
            no_daemon: true,
            ..QueryArgs::new("Cargo.toml")
        };

        assert_eq!(
            default_args.runtime(),
            QueryRuntime::PublishedIndexOnly
        );
        assert_eq!(
            no_daemon_args.runtime(),
            QueryRuntime::PublishedIndexOnly
        );
    }

    #[test]
    fn daemon_query_args_use_daemon_runtime() {
        let args = QueryArgs {
            daemon: true,
            ..QueryArgs::new("Cargo.toml")
        };

        assert_eq!(args.runtime(), QueryRuntime::DaemonRpc);
    }

    #[test]
    fn conflicting_daemon_flags_fail_before_runtime_access() {
        let args = QueryArgs {
            daemon: true,
            no_daemon: true,
            ..QueryArgs::new("Cargo.toml")
        };

        let error = args
            .collect_rows()
            .expect_err("conflicting daemon flags should fail early");

        assert!(
            error
                .to_string()
                .contains("`--daemon` and `--no-daemon` cannot be used together")
        );
    }
}
