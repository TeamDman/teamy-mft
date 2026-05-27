use crate::query::QueryGroup;
use facet::Facet;
use figue::{self as args};

#[derive(Debug, Clone, PartialEq, Default, Facet, arbitrary::Arbitrary)]
pub struct QueryString {
    /// Fast query groups. Each positional argument is `OR`ed; whitespace-delimited terms within one argument are `AND`ed.
    #[facet(args::positional, default)]
    groups: Vec<QueryGroup>,
}

impl QueryString {
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

        let groups = query_inputs
            .iter()
            .flat_map(|query_input| query_input.split('|'))
            .filter_map(QueryGroup::parse)
            .collect::<Vec<_>>();

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
    pub fn to_inputs(&self) -> Vec<String> {
        self.groups.iter().map(ToString::to_string).collect()
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
            '"' | '<' | '>' | '?' | '*' => {
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
