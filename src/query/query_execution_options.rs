use crate::query::QueryIgnoreRules;

#[derive(Debug, Default)]
pub struct QueryExecutionOptions {
    pub ignore: QueryIgnoreBehavior,
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
