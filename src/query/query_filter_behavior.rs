use crate::query::QueryFilterRules;

#[derive(Debug, Default)]
pub enum QueryFilterBehavior {
    #[default]
    AutoDiscover,
    Disabled,
    Custom(QueryFilterRules),
}
