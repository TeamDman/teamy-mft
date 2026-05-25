use crate::query::QueryIgnoreRules;

#[derive(Debug, Default)]
pub struct QueryExecutionOptions {
    pub ignore: QueryIgnoreBehavior,
    pub source: crate::cli::command::query::QuerySource,
}

#[derive(Debug, Default)]
pub enum QueryIgnoreBehavior {
    #[default]
    AutoDiscover,
    Disabled,
    Custom(QueryIgnoreRules),
}
