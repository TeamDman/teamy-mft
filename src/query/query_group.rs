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
    pub fn matches_segments_preprocessed<'a, I, F>(&self, make_segments: &F) -> bool
    where
        I: Iterator<Item = (&'a str, &'a str)>,
        F: Fn() -> I,
    {
        self.rules.iter().all(|rule| {
            make_segments().any(|(segment, normalized_segment)| {
                rule.matches_preprocessed(segment, Some(normalized_segment))
            })
        })
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        self.rules.iter().all(|rule| {
            if let Some(normalized_haystack) = normalized_haystack {
                path_segments(haystack)
                    .zip(path_segments(normalized_haystack))
                    .any(|(segment, normalized_segment)| {
                        rule.matches_preprocessed(segment, Some(normalized_segment))
                    })
            } else {
                path_segments(haystack).any(|segment| rule.matches(segment))
            }
        })
    }
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
}
