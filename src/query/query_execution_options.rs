use crate::query::QueryIgnoreRules;

#[derive(Debug)]
pub struct QueryExecutionOptions {
    pub ignore: QueryIgnoreBehavior,
    pub source: crate::cli::command::query::QuerySource,
}

impl Default for QueryExecutionOptions {
    fn default() -> Self {
        Self {
            ignore: QueryIgnoreBehavior::AutoDiscover,
            source: crate::cli::command::query::QuerySource::Auto,
        }
    }
}

#[derive(Debug)]
pub enum QueryIgnoreBehavior {
    AutoDiscover,
    Disabled,
    Custom(QueryIgnoreRules),
}

impl Default for QueryIgnoreBehavior {
    fn default() -> Self {
        Self::AutoDiscover
    }
}
