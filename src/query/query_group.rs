use crate::query::query_string::validate_query_input;
use crate::query::MatchingRowIndices;
use crate::query::QueryRule;
use arbitrary::Arbitrary;
use facet::Facet;
use std::fmt::Display;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Facet)]
#[facet(opaque, proxy = String)]
pub struct QueryGroup {
    pub rules: Vec<QueryRule>,
}

impl QueryGroup {
    /// # Errors
    ///
    /// Returns an error if any non-empty rule in the group has invalid query
    /// syntax.
    pub fn parse(raw_group: &str) -> eyre::Result<Option<Self>> {
        if raw_group.is_empty() {
            return Ok(None);
        }

        if raw_group.trim().is_empty() {
            return Ok(Some(Self {
                rules: vec![QueryRule::from_str(raw_group)?],
            }));
        }

        let rules = raw_group
            .split_whitespace()
            .map(QueryRule::from_str)
            .collect::<eyre::Result<Vec<_>>>()?;

        if rules.is_empty() {
            return Ok(None);
        }

        Ok(Some(Self { rules }))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
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
            if rule.is_match_all() {
                return true;
            }

            make_segments().any(|(segment, normalized_segment)| {
                rule.matches_preprocessed(segment, Some(normalized_segment))
            })
        })
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        self.rules.iter().all(|rule| {
            if rule.is_match_all() {
                return true;
            }

            if let Some(normalized_haystack) = normalized_haystack {
                if rule.matches_only_terminal_segment() {
                    terminal_path_segment(haystack)
                        .zip(terminal_path_segment(normalized_haystack))
                        .is_some_and(|(segment, normalized_segment)| {
                            rule.matches_preprocessed(segment, Some(normalized_segment))
                        })
                } else {
                    path_segments(haystack)
                        .zip(path_segments(normalized_haystack))
                        .any(|(segment, normalized_segment)| {
                            rule.matches_preprocessed(segment, Some(normalized_segment))
                        })
                }
            } else if rule.matches_only_terminal_segment() {
                terminal_path_segment(haystack).is_some_and(|segment| rule.matches(segment))
            } else {
                path_segments(haystack).any(|segment| rule.matches(segment))
            }
        })
    }

    /// # Errors
    ///
    /// Returns any error produced while looking up candidate rows for a rule.
    pub(crate) fn matching_row_index_candidates<F>(
        &self,
        row_indices_for_rule: &F,
    ) -> eyre::Result<MatchingRowIndices>
    where
        F: Fn(&QueryRule) -> eyre::Result<MatchingRowIndices>,
    {
        let Some((first_rule, remaining_rules)) = self.rules.split_first() else {
            return Ok(MatchingRowIndices::RowIndices(Vec::new()));
        };

        let mut matches = row_indices_for_rule(first_rule)?;

        for rule in remaining_rules {
            matches = matches.intersect(row_indices_for_rule(rule)?);
            if matches.is_empty() {
                break;
            }
        }

        Ok(matches)
    }
}

impl Display for QueryGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, rule) in self.rules.iter().enumerate() {
            if index > 0 {
                write!(f, " ")?;
            }
            write!(f, "{rule}")?;
        }
        Ok(())
    }
}

impl TryFrom<String> for QueryGroup {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_query_input(&value).map_err(|error| error.to_string())?;
        Self::parse(&value)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "query group cannot be empty".to_owned())
    }
}

impl From<QueryGroup> for String {
    fn from(value: QueryGroup) -> Self {
        value.to_string()
    }
}

impl From<&QueryGroup> for String {
    fn from(value: &QueryGroup) -> Self {
        value.to_string()
    }
}

impl<'a> Arbitrary<'a> for QueryGroup {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let input = String::arbitrary(u)?;
        Ok(if validate_query_input(&input).is_ok() {
            Self::parse(&input).ok().flatten()
        } else {
            None
        }
        .unwrap_or_else(|| {
            Self::parse("query")
                .expect("fallback query should parse")
                .expect("fallback query should produce a group")
        }))
    }
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
}

fn terminal_path_segment(path: &str) -> Option<&str> {
    path_segments(path).last()
}
