use crate::query::QueryPath;

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct IndexedPathRow {
    pub path: QueryPath,
    pub has_deleted_entries: bool,
    pub is_ignored: bool,
}
