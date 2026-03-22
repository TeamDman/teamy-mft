use crate::query::QueryRule;

#[derive(Debug, Clone)]
pub struct QueryGroup {
    rules: Vec<QueryRule>,
}

impl QueryGroup {
    pub fn parse(raw_group: &str) -> Option<Self> {
        let rules = raw_group
            .split_whitespace()
            .filter_map(QueryRule::parse)
            .collect::<Vec<_>>();

        if rules.is_empty() {
            return None;
        }

        Some(Self { rules })
    }

    #[must_use]
    pub fn matches(&self, haystack: &str) -> bool {
        self.matches_preprocessed(haystack, None)
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        self.rules
            .iter()
            .all(|rule| rule.matches_preprocessed(haystack, normalized_haystack))
    }
}
