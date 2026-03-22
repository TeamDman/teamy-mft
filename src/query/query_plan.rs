use crate::query::QueryGroup;

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
    /// Returns an error if no non-empty query groups are present after parsing.
    pub fn parse_inputs(query_inputs: &[String]) -> eyre::Result<Self> {
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
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        self.groups
            .iter()
            .any(|group| group.matches_preprocessed(haystack, normalized_haystack))
    }
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
    fn leading_quote_marker_is_ignored_within_terms() {
        assert_eq!(
            matching_paths(
                &["'flower .jar$", "trees"],
                &["FLOWER.jar", "trees.zip", "jarflower.txt"]
            ),
            vec!["FLOWER.jar", "trees.zip"]
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
}
