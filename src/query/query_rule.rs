use crate::query::QueryNeedle;
use crate::query::query_needle::QUERY_TRIGRAM_LEN;
use std::fmt::Display;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum QueryRule {
    MatchAll,
    PrefixCaseInsensitive(QueryNeedle),
    ContainsCaseInsensitive(QueryNeedle),
    EndsWithCaseInsensitive(QueryNeedle),
    EqualsCaseInsensitive(QueryNeedle),
}

impl QueryRule {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::MatchAll => false,
            Self::PrefixCaseInsensitive(needle)
            | Self::ContainsCaseInsensitive(needle)
            | Self::EndsWithCaseInsensitive(needle)
            | Self::EqualsCaseInsensitive(needle) => needle.is_empty(),
        }
    }

    #[must_use]
    pub fn is_match_all(&self) -> bool {
        matches!(self, Self::MatchAll)
    }

    #[must_use]
    pub fn matches(&self, haystack: &str) -> bool {
        self.matches_preprocessed(haystack, None)
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        match self {
            Self::MatchAll => true,
            Self::PrefixCaseInsensitive(needle) => {
                needle.matches_prefix_preprocessed(haystack, normalized_haystack)
            }
            Self::ContainsCaseInsensitive(needle) => {
                needle.matches_contains_preprocessed(haystack, normalized_haystack)
            }
            Self::EndsWithCaseInsensitive(needle) => {
                needle.matches_suffix_preprocessed(haystack, normalized_haystack)
            }
            Self::EqualsCaseInsensitive(needle) => {
                needle.matches_exact_preprocessed(haystack, normalized_haystack)
            }
        }
    }

    #[must_use]
    pub fn matches_normalized(&self, normalized_haystack: &str) -> bool {
        self.matches_preprocessed(normalized_haystack, Some(normalized_haystack))
    }

    #[must_use]
    pub fn matches_only_terminal_segment(&self) -> bool {
        matches!(
            self,
            Self::EndsWithCaseInsensitive(_) | Self::EqualsCaseInsensitive(_)
        )
    }

    #[must_use]
    pub fn normalized_extension_suffix(&self) -> Option<&str> {
        match self {
            Self::MatchAll => None,
            Self::EndsWithCaseInsensitive(needle) => {
                let suffix = needle.normalized_str();
                (suffix.starts_with('.') && suffix.len() > 1).then_some(suffix)
            }
            Self::PrefixCaseInsensitive(_)
            | Self::ContainsCaseInsensitive(_)
            | Self::EqualsCaseInsensitive(_) => None,
        }
    }

    #[must_use]
    pub fn normalized_contains_trigrams(&self) -> Option<Vec<[u8; QUERY_TRIGRAM_LEN]>> {
        match self {
            Self::MatchAll => None,
            Self::ContainsCaseInsensitive(needle)
                if needle.normalized_bytes().len() >= QUERY_TRIGRAM_LEN =>
            {
                Some(needle.normalized_trigrams())
            }
            Self::PrefixCaseInsensitive(_)
            | Self::ContainsCaseInsensitive(_)
            | Self::EndsWithCaseInsensitive(_)
            | Self::EqualsCaseInsensitive(_) => None,
        }
    }
}

impl FromStr for QueryRule {
    type Err = eyre::Error;

    fn from_str(raw_term: &str) -> Result<Self, Self::Err> {
        if raw_term.is_empty() {
            eyre::bail!("query rule cannot be empty");
        }

        if raw_term == "<>" {
            return Ok(Self::MatchAll);
        }

        if let Some(inner) = raw_term.strip_prefix('<') {
            if let Some(exact) = inner.strip_suffix('>') {
                if exact.is_empty() {
                    eyre::bail!("exact query rule cannot be empty");
                }
                return Ok(Self::EqualsCaseInsensitive(QueryNeedle::new(exact)));
            }

            if inner.is_empty() {
                eyre::bail!("prefix query rule cannot be empty");
            }
            return Ok(Self::PrefixCaseInsensitive(QueryNeedle::new(inner)));
        }

        if let Some(suffix) = raw_term.strip_suffix('>') {
            if suffix.is_empty() {
                eyre::bail!("suffix query rule cannot be empty");
            }
            return Ok(Self::EndsWithCaseInsensitive(QueryNeedle::new(suffix)));
        }

        Ok(Self::ContainsCaseInsensitive(QueryNeedle::new(raw_term)))
    }
}

impl Display for QueryRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MatchAll => write!(f, "<>"),
            Self::PrefixCaseInsensitive(needle) => write!(f, "<{}", needle.normalized_str()),
            Self::ContainsCaseInsensitive(needle) => write!(f, "{}", needle.normalized_str()),
            Self::EndsWithCaseInsensitive(needle) => write!(f, "{}>", needle.normalized_str()),
            Self::EqualsCaseInsensitive(needle) => write!(f, "<{}>", needle.normalized_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::QueryRule;
    use std::str::FromStr;

    #[test]
    fn empty_rule_reports_a_helpful_error() {
        let error = QueryRule::from_str("").expect_err("empty rules should be rejected");
        assert!(error.to_string().contains("query rule cannot be empty"));
    }

    #[test]
    fn empty_prefix_rule_reports_a_helpful_error() {
        let error = QueryRule::from_str("<").expect_err("empty prefix should be rejected");
        assert!(
            error
                .to_string()
                .contains("prefix query rule cannot be empty")
        );
    }

    #[test]
    fn empty_suffix_rule_reports_a_helpful_error() {
        let error = QueryRule::from_str(">").expect_err("empty suffix should be rejected");
        assert!(
            error
                .to_string()
                .contains("suffix query rule cannot be empty")
        );
    }

    #[test]
    fn empty_exact_syntax_now_parses_as_match_all() {
        let rule = QueryRule::from_str("<>").expect("match-all rule should parse");

        assert_eq!(rule, QueryRule::MatchAll);
        assert!(rule.matches("anything"));
        assert_eq!(rule.to_string(), "<>");
    }
}
