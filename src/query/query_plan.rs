use crate::query::QueryGroup;
use crate::query::QueryRule;

#[derive(Debug, Clone)]
pub struct QueryPlan {
    groups: Vec<QueryGroup>,
}

impl QueryPlan {
    /// Build a query plan from CLI positional inputs.
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
            eyre::bail!("query string required")
        }

        Ok(Self { groups })
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
    /// rules in this plan.
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

fn validate_query_input(query_input: &str) -> eyre::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::QueryPlan;

    fn matching_paths(query_inputs: &[&str], paths: &[&str]) -> Vec<String> {
        let query_inputs = query_inputs
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        let plan = QueryPlan::parse_inputs(&query_inputs).expect("query should parse");
        paths
            .iter()
            .copied()
            .filter(|path| plan.matches(path))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn suffix_rule_matches_extension_case_insensitively() {
        assert_eq!(
            matching_paths(&[".webm$"], &["a.txt", "c.WEBM"]),
            vec!["c.WEBM"]
        );
    }

    #[test]
    fn contains_rule_matches_case_insensitively() {
        assert_eq!(
            matching_paths(&["flower"], &["FLOWER.jar", "trees.zip"]),
            vec!["FLOWER.jar"]
        );
    }

    #[test]
    fn whitespace_separated_terms_are_anded_within_one_group() {
        assert_eq!(
            matching_paths(
                &["flower .jar$"],
                &["flower.jar", "flower.zip", "other.jar"]
            ),
            vec!["flower.jar"]
        );
    }

    #[test]
    fn repeated_positional_args_are_ored() {
        assert_eq!(
            matching_paths(
                &["flower .jar$", "trees.zip"],
                &["flower.jar", "trees.zip", "other.bin"]
            ),
            vec!["flower.jar", "trees.zip"]
        );
    }

    #[test]
    fn pipe_separator_within_one_argument_is_ored() {
        assert_eq!(
            matching_paths(
                &["flower .jar$ | trees.zip"],
                &["flower.jar", "trees.zip", "other.bin"]
            ),
            vec!["flower.jar", "trees.zip"]
        );
    }

    #[test]
    fn pipes_and_argument_array_unify_to_the_same_plan() {
        let via_pipe = matching_paths(
            &["flower .jar$ | trees.zip"],
            &["flower.jar", "trees.zip", "other.bin"],
        );
        let via_args = matching_paths(
            &["flower .jar$", "trees.zip"],
            &["flower.jar", "trees.zip", "other.bin"],
        );
        assert_eq!(via_pipe, via_args);
    }

    #[test]
    fn apostrophe_is_treated_as_a_literal_query_character() {
        assert_eq!(
            matching_paths(
                &["o'connor .txt$", "trees"],
                &["O'Connor.txt", "oconnor.txt", "trees.zip"]
            ),
            vec!["O'Connor.txt", "trees.zip"]
        );
    }

    #[test]
    fn multiple_pipe_segments_and_blank_segments_are_ignored() {
        assert_eq!(
            matching_paths(
                &[" | flower .jar$ |  | trees.zip | "],
                &["flower.jar", "trees.zip", "other.bin"]
            ),
            vec!["flower.jar", "trees.zip"]
        );
    }

    #[test]
    fn postings_candidate_rows_follow_or_of_ands_semantics() {
        let plan = QueryPlan::parse_inputs(&[String::from("alpha beta"), String::from("gamma")])
            .expect("query should parse");

        let candidates = plan
            .matching_row_indices(&|rule| {
                Ok(match format!("{rule:?}").as_str() {
                    "ContainsCaseInsensitive(AsciiLower([97, 108, 112, 104, 97]))" => {
                        vec![0, 1, 3]
                    }
                    "ContainsCaseInsensitive(AsciiLower([98, 101, 116, 97]))" => vec![1, 2, 3],
                    "ContainsCaseInsensitive(AsciiLower([103, 97, 109, 109, 97]))" => vec![5],
                    other => panic!("unexpected rule: {other}"),
                })
            })
            .expect("candidate lookup should succeed");

        assert_eq!(candidates, vec![1, 3, 5]);
    }

    #[test]
    fn empty_inputs_are_rejected() {
        let query_inputs = vec!["   ".to_owned(), "|".to_owned()];
        assert!(QueryPlan::parse_inputs(&query_inputs).is_err());
    }

    #[test]
    fn preprocessed_normalized_haystack_matches_the_same_result() {
        let query_inputs = vec!["FLOWER .jar$".to_owned()];
        let plan = QueryPlan::parse_inputs(&query_inputs).expect("query should parse");

        assert!(plan.matches_preprocessed("Flower.JAR", Some("flower.jar")));
        assert!(!plan.matches_preprocessed("Flower.ZIP", Some("flower.zip")));
    }

    #[test]
    fn rules_match_against_individual_path_segments() {
        assert_eq!(
            matching_paths(
                &["a b c"],
                &[
                    "a b c.txt",
                    "abc.txt",
                    "bca.txt",
                    "a/b/d/c.txt",
                    "a/d/e.txt"
                ]
            ),
            vec!["a b c.txt", "abc.txt", "bca.txt", "a/b/d/c.txt"]
        );
    }

    #[test]
    fn rule_does_not_match_across_path_separators() {
        assert_eq!(
            matching_paths(&["alpha/beta"], &["alpha/beta.txt", "alpha-beta.txt"]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn suffix_rules_apply_only_to_terminal_segments() {
        assert_eq!(
            matching_paths(&[".txt$"], &["a/b/c.txt", "a/b/c.zip", "a/.txt/c.zip"]),
            vec!["a/b/c.txt"]
        );
    }

    #[test]
    fn suffix_rules_do_not_match_non_terminal_segments() {
        assert_eq!(
            matching_paths(
                &[".git$"],
                &[
                    "repo/project.git",
                    "repo/.git/objects/pack/pack-a.rev",
                    "repo/.git/refs/remotes/origin/main"
                ]
            ),
            vec!["repo/project.git"]
        );
    }

    #[test]
    fn windows_invalid_query_characters_are_rejected_eagerly() {
        let query_inputs = vec!["flower?.jar".to_owned()];
        let error = QueryPlan::parse_inputs(&query_inputs).expect_err("query should be rejected");
        assert!(
            error
                .to_string()
                .contains("Windows-invalid path character '?'")
        );
    }

    #[test]
    fn double_quote_is_rejected_eagerly() {
        let query_inputs = vec!["flower\".jar".to_owned()];
        let error = QueryPlan::parse_inputs(&query_inputs).expect_err("query should be rejected");
        assert!(
            error
                .to_string()
                .contains("Windows-invalid path character '\"'")
        );
    }

    #[test]
    fn colon_is_rejected_outside_a_drive_designator() {
        let query_inputs = vec!["flower:jar".to_owned()];
        let error = QueryPlan::parse_inputs(&query_inputs).expect_err("query should be rejected");
        assert!(
            error
                .to_string()
                .contains("unsupported ':' outside a drive designator")
        );
    }

    #[test]
    fn drive_designators_are_allowed_in_queries() {
        let query_inputs = vec!["C:\\src .txt$".to_owned()];
        QueryPlan::parse_inputs(&query_inputs).expect("query should parse");
    }
}
