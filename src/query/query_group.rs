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

    /// # Errors
    ///
    /// Returns any error produced while looking up candidate rows for a rule.
    pub fn matching_row_indices<F>(&self, row_indices_for_rule: &F) -> eyre::Result<Vec<u32>>
    where
        F: Fn(&QueryRule) -> eyre::Result<Vec<u32>>,
    {
        let Some((first_rule, remaining_rules)) = self.rules.split_first() else {
            return Ok(Vec::new());
        };

        let mut matches = row_indices_for_rule(first_rule)?;
        matches.sort_unstable();
        matches.dedup();

        for rule in remaining_rules {
            let mut rule_matches = row_indices_for_rule(rule)?;
            rule_matches.sort_unstable();
            rule_matches.dedup();
            matches = intersect_sorted(&matches, &rule_matches);
            if matches.is_empty() {
                break;
            }
        }

        Ok(matches)
    }
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
}

fn intersect_sorted(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut left_index = 0;
    let mut right_index = 0;
    let mut intersection = Vec::with_capacity(left.len().min(right.len()));

    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                intersection.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }

    intersection
}
