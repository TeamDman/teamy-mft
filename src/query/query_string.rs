use crate::query::QueryGroup;
use crate::query::QueryRule;
use facet::Facet;
use figue::{self as args};

#[derive(Debug, Clone, PartialEq, Facet, arbitrary::Arbitrary)]
pub struct QueryString {
    /// Fast query groups. Each positional argument is `OR`ed; whitespace-delimited terms within one argument are `AND`ed.
    #[facet(args::positional, default = QueryString::default().groups)]
    pub groups: Vec<QueryGroup>,
}
impl Default for QueryString {
    fn default() -> Self {
        Self::single_rule(QueryRule::MatchAll)
    }
}

impl QueryString {
    pub fn single_rule(rule: QueryRule) -> Self {
        Self {
            groups: vec![QueryGroup { rules: vec![rule] }],
        }
    }
    /// Build a query string from CLI positional inputs.
    ///
    /// Each positional argument is treated as an `OR` group, and `|` inside any
    /// positional argument is normalized into the same `OR` grouping model.
    ///
    /// # Errors
    ///
    /// Returns an error if no non-empty query groups are present after parsing,
    /// or if any raw query input contains Windows-invalid path characters that
    /// are not part of the recognized query syntax.
    pub fn parse_inputs(query_inputs: &[String]) -> eyre::Result<Self> {
        for query_input in query_inputs {
            validate_query_input(query_input)?;
        }

        let mut groups = Vec::new();
        for raw_group in query_inputs
            .iter()
            .flat_map(|query_input| query_input.split('|'))
        {
            if let Some(group) = QueryGroup::parse(raw_group)? {
                groups.push(group);
            }
        }

        if groups.is_empty() {
            eyre::bail!("query string required");
        }

        Ok(Self { groups })
    }

    #[must_use]
    pub fn groups(&self) -> &[QueryGroup] {
        &self.groups
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    #[must_use]
    pub fn to_inputs(&self) -> Vec<String> {
        self.groups.iter().map(ToString::to_string).collect()
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
        self.groups
            .iter()
            .any(|group| group.matches_segments_preprocessed(make_segments))
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        self.groups
            .iter()
            .any(|group| group.matches_preprocessed(haystack, normalized_haystack))
    }

    /// # Errors
    ///
    /// Returns any error produced while looking up candidate rows for the
    /// rules in this query string.
    pub fn matching_row_indices<F>(&self, row_indices_for_rule: &F) -> eyre::Result<Vec<u32>>
    where
        F: Fn(&QueryRule) -> eyre::Result<Vec<u32>>,
    {
        let mut matches = Vec::new();

        for group in &self.groups {
            matches.extend(group.matching_row_indices(row_indices_for_rule)?);
        }

        matches.sort_unstable();
        matches.dedup();
        Ok(matches)
    }
}

impl From<QueryString> for Vec<String> {
    fn from(value: QueryString) -> Self {
        value.to_inputs()
    }
}

impl From<&QueryString> for Vec<String> {
    fn from(value: &QueryString) -> Self {
        value.to_inputs()
    }
}

pub(crate) fn validate_query_input(query_input: &str) -> eyre::Result<()> {
    let chars = query_input.chars().collect::<Vec<_>>();

    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_control() {
            eyre::bail!(
                "query contains unsupported control character {:?} in {:?}",
                ch,
                query_input
            );
        }

        match ch {
            '"' | '?' | '*' => {
                // | and < and > are special characters used in our query parsing logic
                eyre::bail!(
                    "query contains Windows-invalid path character {:?} in {:?}",
                    ch,
                    query_input
                );
            }
            ':' if !is_drive_designator(&chars, index) => {
                eyre::bail!(
                    "query contains unsupported ':' outside a drive designator in {:?}",
                    query_input
                );
            }
            _ => {}
        }
    }

    Ok(())
}

fn is_drive_designator(chars: &[char], colon_index: usize) -> bool {
    if colon_index == 0 || !chars[colon_index - 1].is_ascii_alphabetic() {
        return false;
    }

    let is_left_boundary = match colon_index
        .checked_sub(2)
        .and_then(|index| chars.get(index))
    {
        None => true,
        Some(ch) => is_query_boundary(*ch),
    };
    let is_right_boundary = match chars.get(colon_index + 1) {
        None => true,
        Some(ch) => is_query_boundary(*ch),
    };

    is_left_boundary && is_right_boundary
}

fn is_query_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '|' | '/' | '\\' | '\'')
}

#[cfg(test)]
mod tests {
    use super::QueryString;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;

    #[test]
    fn query_string_intersects_contains_and_suffix_candidates() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\src\\flower.jar").into(),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\flowchart.txt").into(),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\trees.zip").into(),
                has_deleted_entries: false,
            },
        ];
        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        let parsed = SearchIndexBytes::new(bytes).parse_trusted_for_query()?;
        let query = QueryString::parse_inputs(&[String::from("flow .jar>")])?;

        assert_eq!(
            query.matching_row_indices(&|rule| crate::query::matching_row_indices_for_rule(
                &parsed, rule
            ))?,
            vec![0]
        );

        Ok(())
    }
}
