use crate::query::QueryFilterRules;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::QueryScope;
use crate::query::resolve_query_scopes;
use std::path::Path;

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Query filter flags mirror independent CLI/RPC filtering switches"
)]
pub struct QueryRowFilter {
    scopes: Vec<QueryScope>,
    filter_rules: Option<QueryFilterRules>,
    include_deleted: bool,
    only_deleted: bool,
    show_filtered: bool,
    only_filtered: bool,
}

impl QueryRowFilter {
    /// # Errors
    ///
    /// Returns an error if the query scope cannot be canonicalized.
    pub fn new(request: &QueryPlan, filter_rules: Option<QueryFilterRules>) -> eyre::Result<Self> {
        Ok(Self {
            scopes: resolve_query_scopes(&request.r#in)?,
            filter_rules,
            include_deleted: request.include_deleted,
            only_deleted: request.only_deleted,
            show_filtered: request.show_filtered,
            only_filtered: request.only_filtered,
        })
    }

    #[must_use]
    pub fn include_deleted_state(&self, has_deleted_entries: bool) -> bool {
        if self.only_deleted {
            return has_deleted_entries;
        }

        self.include_deleted || !has_deleted_entries
    }

    #[must_use]
    pub fn include_filtered_state(&self, is_filtered: bool) -> bool {
        if self.only_filtered {
            return is_filtered;
        }

        self.show_filtered || !is_filtered
    }

    #[must_use]
    pub fn matches_scope(&self, path: &Path) -> bool {
        self.scopes.is_empty() || self.scopes.iter().any(|scope| scope.matches_path(path))
    }

    #[must_use]
    pub(crate) fn scopes(&self) -> &[QueryScope] {
        &self.scopes
    }

    #[must_use]
    pub fn classify_and_match(&self, mut row: QueryResultRow) -> Option<QueryResultRow> {
        if !self.include_deleted_state(row.has_deleted_entries) {
            return None;
        }
        if !self.matches_scope(row.path.as_path()) {
            return None;
        }

        row.is_filtered = self
            .filter_rules
            .as_ref()
            .is_some_and(|rules| rules.is_filtered_path(row.path.as_path()));

        self.include_filtered_state(row.is_filtered).then_some(row)
    }
}

#[cfg(test)]
mod tests {
    use super::QueryRowFilter;
    use crate::query::Pathlike;
    use crate::query::QueryPlan;
    use crate::query::QueryResultRow;

    fn request() -> QueryPlan {
        QueryPlan::new("music")
    }

    fn row(has_deleted_entries: bool) -> QueryResultRow {
        QueryResultRow {
            path: Pathlike::from(String::from(r"C:\music\track.flac")),
            has_deleted_entries,
            is_filtered: false,
        }
    }

    #[test]
    fn hides_deleted_rows_by_default() {
        let filter = QueryRowFilter::new(&request(), None).expect("filter should build");

        assert!(filter.classify_and_match(row(false)).is_some());
        assert!(filter.classify_and_match(row(true)).is_none());
    }

    #[test]
    fn only_deleted_filters_to_deleted_rows() {
        let filter = QueryRowFilter::new(
            &QueryPlan {
                only_deleted: true,
                ..request()
            },
            None,
        )
        .expect("filter should build");

        assert!(filter.classify_and_match(row(true)).is_some());
        assert!(filter.classify_and_match(row(false)).is_none());
    }

    #[test]
    fn filtered_rows_are_hidden_by_default() {
        let filter = QueryRowFilter::new(&request(), None).expect("filter should build");

        assert!(!filter.include_filtered_state(true));
        assert!(filter.include_filtered_state(false));
    }

    #[test]
    fn show_filtered_includes_both_visible_and_filtered_rows() {
        let filter = QueryRowFilter::new(
            &QueryPlan {
                show_filtered: true,
                ..request()
            },
            None,
        )
        .expect("filter should build");

        assert!(filter.include_filtered_state(true));
        assert!(filter.include_filtered_state(false));
    }

    #[test]
    fn only_filtered_filters_to_filtered_rows() {
        let filter = QueryRowFilter::new(
            &QueryPlan {
                only_filtered: true,
                ..request()
            },
            None,
        )
        .expect("filter should build");

        assert!(filter.include_filtered_state(true));
        assert!(!filter.include_filtered_state(false));
    }
}
