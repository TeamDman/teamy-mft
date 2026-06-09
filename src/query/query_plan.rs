use crate::query::DEFAULT_PROFILE_NAME;
use crate::query::QueryGroup;
use crate::query::QueryLimit;
use crate::query::QueryRule;
use crate::query::QueryString;
use crate::query::normalize_profile_name;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
#[facet(rename_all = "kebab-case")]
// cli[impl command.query.drive-pattern-selection]
#[allow(
    clippy::struct_excessive_bools,
    reason = "CLI flags map directly to independent query toggles"
)]
pub struct QueryPlan {
    #[facet(flatten, default)]
    pub query: QueryString,
    /// Restrict results to this path. Directories include descendants; files match exactly.
    #[facet(args::named, default)]
    pub r#in: Option<String>,
    /// Apply profile-specific `.teamy_mft_rules` files in addition to global rules.
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Drive letter pattern to match drives whose cached MFTs will be queried (e.g., "*", "C", "CD", "C,D"). Compatibility alias: `--drive`.
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Maximum number of results to show
    #[facet(args::named, default)]
    pub limit: QueryLimit,
    /// Include paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub include_deleted: bool,
    /// Show only paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub only_deleted: bool,
    /// Include paths filtered out by `.teamy_mft_rules` filter rules
    #[facet(args::named, default)]
    pub show_filtered: bool,
    /// Show only paths filtered out by `.teamy_mft_rules` filter rules
    #[facet(args::named, default)]
    pub only_filtered: bool,
}

impl QueryPlan {
    #[must_use]
    pub fn single_rule(rule: QueryRule) -> Self {
        Self {
            query: QueryString {
                groups: vec![QueryGroup { rules: vec![rule] }],
            },
            ..Default::default()
        }
    }

    /// Build a query plan from CLI positional inputs.
    ///
    /// # Errors
    ///
    /// Returns an error if any query input cannot be parsed into a valid query
    /// string.
    pub fn parse_inputs(query_inputs: &[String]) -> eyre::Result<Self> {
        Ok(Self {
            query: QueryString::parse_inputs(query_inputs)?,
            ..Default::default()
        })
    }

    /// Create a new `QueryPlan` with the given query pattern and all other options at their defaults.
    ///
    /// # Panics
    ///
    /// Panics if the provided pattern is not a valid single query.
    pub fn new(pattern: impl Into<String>) -> Self {
        Self::parse_inputs(&[pattern.into()]).expect("single non-empty query should parse")
    }

    /// # Errors
    ///
    /// Returns an error if the current process is not allowed to use the
    /// logical default profile or if the selected profile name is invalid.
    pub fn ensure_selected_profile_allowed(&self) -> eyre::Result<()> {
        let current_sid = crate::machine::security::current_user_sid_string()?;
        let machine_config = crate::machine::config::load_machine_client_config()?;
        let is_in_builtin_administrators =
            crate::windows_utils::elevation::is_in_builtin_administrators()?;
        self.ensure_selected_profile_allowed_for_identity(
            machine_config.owner_sid.as_str(),
            current_sid.as_str(),
            is_in_builtin_administrators,
        )
    }

    fn ensure_selected_profile_allowed_for_identity(
        &self,
        owner_sid: &str,
        current_sid: &str,
        is_in_builtin_administrators: bool,
    ) -> eyre::Result<()> {
        if normalize_profile_name(self.profile.as_deref())?.is_some()
            || (!owner_sid.is_empty() && owner_sid == current_sid)
            || is_in_builtin_administrators
        {
            return Ok(());
        }
        eyre::bail!(
            "The {} profile is disabled for queries started by users other than the installed machine-cache owner unless the current token is in BUILTIN\\Administrators. This usually means the query is running in a sandboxed or restricted account. Re-run the query with `--profile <name>`.",
            DEFAULT_PROFILE_NAME
        );
    }
}

#[cfg(test)]
mod tests {
    use super::QueryPlan;
    use crate::query::DEFAULT_PROFILE_NAME;
    use crate::query::QueryNeedle;
    use crate::query::QueryRule;

    fn matching_paths(query_inputs: &[&str], paths: &[&str]) -> Vec<String> {
        let query_inputs = query_inputs
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        let plan = QueryPlan::parse_inputs(&query_inputs).expect("query should parse");
        paths
            .iter()
            .copied()
            .filter(|path| plan.query.matches(path))
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
            .query
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
        let query_inputs = vec![String::new(), "|".to_owned()];
        assert!(QueryPlan::parse_inputs(&query_inputs).is_err());
    }

    #[test]
    fn whitespace_only_inputs_are_preserved_as_literal_queries() {
        let query_inputs = vec!["   ".to_owned()];
        let plan = QueryPlan::parse_inputs(&query_inputs).expect("query should parse");

        assert!(plan.query.matches("alpha   beta"));
        assert!(!plan.query.matches("alphabet"));
    }

    #[test]
    fn preprocessed_normalized_haystack_matches_the_same_result() {
        let query_inputs = vec!["FLOWER .jar$".to_owned()];
        let plan = QueryPlan::parse_inputs(&query_inputs).expect("query should parse");

        assert!(
            plan.query
                .matches_preprocessed("Flower.JAR", Some("flower.jar"))
        );
        assert!(
            !plan
                .query
                .matches_preprocessed("Flower.ZIP", Some("flower.zip"))
        );
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

    #[test]
    fn single_rule_builds_one_group_with_one_rule() {
        let plan =
            QueryPlan::single_rule(QueryRule::EndsWithCaseInsensitive(QueryNeedle::new(".jar")));

        assert_eq!(plan.query.groups.len(), 1);
        assert_eq!(plan.query.groups[0].rules.len(), 1);
        assert!(plan.query.matches("flower.jar"));
        assert!(!plan.query.matches("flower.zip"));
    }

    #[test]
    fn default_profile_is_rejected_for_non_admin_tokens() {
        let plan = QueryPlan::new("flower");
        let error = plan
            .ensure_selected_profile_allowed_for_identity("S-1-5-21-owner", "S-1-5-21-other", false)
            .expect_err("default profile should be blocked outside administrators");
        assert!(error.to_string().contains(DEFAULT_PROFILE_NAME));
        assert!(error.to_string().contains("`--profile <name>`"));
    }

    #[test]
    fn default_profile_alias_is_rejected_for_non_admin_tokens() {
        let plan = QueryPlan {
            profile: Some(DEFAULT_PROFILE_NAME.to_owned()),
            ..QueryPlan::new("flower")
        };
        let error = plan
            .ensure_selected_profile_allowed_for_identity("S-1-5-21-owner", "S-1-5-21-other", false)
            .expect_err("default alias should be blocked outside administrators");
        assert!(error.to_string().contains(DEFAULT_PROFILE_NAME));
    }

    #[test]
    fn named_profile_is_allowed_for_non_admin_tokens() {
        let plan = QueryPlan {
            profile: Some(String::from("music")),
            ..QueryPlan::new("flower")
        };
        plan.ensure_selected_profile_allowed_for_identity(
            "S-1-5-21-owner",
            "S-1-5-21-other",
            false,
        )
        .expect("named profiles should stay available outside administrators");
    }

    #[test]
    fn default_profile_is_allowed_for_admin_tokens() {
        QueryPlan::new("flower")
            .ensure_selected_profile_allowed_for_identity("S-1-5-21-owner", "S-1-5-21-other", true)
            .expect("administrators should keep access to the default profile");
    }

    #[test]
    fn default_profile_is_allowed_for_machine_cache_owner() {
        QueryPlan::new("flower")
            .ensure_selected_profile_allowed_for_identity("S-1-5-21-owner", "S-1-5-21-owner", false)
            .expect("the installed machine-cache owner should keep access to the default profile");
    }
}
