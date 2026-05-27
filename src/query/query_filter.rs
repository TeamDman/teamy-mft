use crate::query::IndexedPathRow;
use crate::query::QueryIgnoreRules;
use crate::query::QueryRequestSpec;
use crate::query::QueryScope;
use crate::query::resolve_query_scope;
use std::path::Path;

#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Query filter flags mirror independent CLI/RPC filtering switches"
)]
pub struct QueryFilter {
    scope: Option<QueryScope>,
    ignore_rules: Option<QueryIgnoreRules>,
    include_deleted: bool,
    only_deleted: bool,
    show_ignored: bool,
    only_ignored: bool,
}

impl QueryFilter {
    /// # Errors
    ///
    /// Returns an error if the query scope cannot be canonicalized.
    pub fn new(
        spec: &QueryRequestSpec,
        ignore_rules: Option<QueryIgnoreRules>,
    ) -> eyre::Result<Self> {
        Ok(Self {
            scope: resolve_query_scope(spec.query_scope.as_deref())?,
            ignore_rules,
            include_deleted: spec.include_deleted,
            only_deleted: spec.only_deleted,
            show_ignored: spec.show_ignored,
            only_ignored: spec.only_ignored,
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
    pub fn include_ignored_state(&self, is_ignored: bool) -> bool {
        if self.only_ignored {
            return is_ignored;
        }

        self.show_ignored || !is_ignored
    }

    #[must_use]
    pub fn matches_scope(&self, path: &Path) -> bool {
        self.scope
            .as_ref()
            .is_none_or(|scope| scope.matches_path(path))
    }

    #[must_use]
    pub fn classify_and_match(&self, mut row: IndexedPathRow) -> Option<IndexedPathRow> {
        if !self.include_deleted_state(row.has_deleted_entries) {
            return None;
        }
        if !self.matches_scope(row.path.as_path()) {
            return None;
        }

        row.is_ignored = self
            .ignore_rules
            .as_ref()
            .is_some_and(|rules| rules.is_ignored_path(row.path.as_path()));

        self.include_ignored_state(row.is_ignored).then_some(row)
    }
}

#[cfg(test)]
mod tests {
    use super::QueryFilter;
    use crate::query::IndexedPathRow;
    use crate::query::QueryRequestSpec;
    use crate::query::QueryResultPath;
    use crate::query::QuerySource;

    fn spec() -> QueryRequestSpec {
        QueryRequestSpec {
            query: vec![String::from("music")],
            query_scope: None,
            drive_letters: vec!['C'],
            limit: 0,
            include_deleted: false,
            only_deleted: false,
            show_ignored: false,
            only_ignored: false,
            source: QuerySource::DiskOnly,
            allow_fallback: false,
        }
    }

    fn row(has_deleted_entries: bool) -> IndexedPathRow {
        IndexedPathRow {
            path: QueryResultPath::from(String::from(r"C:\music\track.flac")),
            has_deleted_entries,
            is_ignored: false,
        }
    }

    #[test]
    fn hides_deleted_rows_by_default() {
        let filter = QueryFilter::new(&spec(), None).expect("filter should build");

        assert!(filter.classify_and_match(row(false)).is_some());
        assert!(filter.classify_and_match(row(true)).is_none());
    }

    #[test]
    fn only_deleted_filters_to_deleted_rows() {
        let filter = QueryFilter::new(
            &QueryRequestSpec {
                only_deleted: true,
                ..spec()
            },
            None,
        )
        .expect("filter should build");

        assert!(filter.classify_and_match(row(true)).is_some());
        assert!(filter.classify_and_match(row(false)).is_none());
    }

    #[test]
    fn ignored_rows_are_hidden_by_default() {
        let filter = QueryFilter::new(&spec(), None).expect("filter should build");

        assert!(!filter.include_ignored_state(true));
        assert!(filter.include_ignored_state(false));
    }

    #[test]
    fn show_ignored_includes_both_visible_and_ignored_rows() {
        let filter = QueryFilter::new(
            &QueryRequestSpec {
                show_ignored: true,
                ..spec()
            },
            None,
        )
        .expect("filter should build");

        assert!(filter.include_ignored_state(true));
        assert!(filter.include_ignored_state(false));
    }

    #[test]
    fn only_ignored_filters_to_ignored_rows() {
        let filter = QueryFilter::new(
            &QueryRequestSpec {
                only_ignored: true,
                ..spec()
            },
            None,
        )
        .expect("filter should build");

        assert!(filter.include_ignored_state(true));
        assert!(!filter.include_ignored_state(false));
    }
}
