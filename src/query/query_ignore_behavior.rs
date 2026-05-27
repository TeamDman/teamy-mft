use crate::query::QueryIgnoreRules;

#[derive(Debug, Default)]
pub enum QueryIgnoreBehavior {
    #[default]
    AutoDiscover,
    Disabled,
    Custom(QueryIgnoreRules),
}
