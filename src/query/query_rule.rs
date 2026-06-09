use crate::query::QueryNeedle;
use crate::query::query_needle::QUERY_TRIGRAM_LEN;
use std::fmt::Display;

#[derive(Debug, Clone, PartialEq)]
pub enum QueryRule {
    PrefixCaseInsensitive(QueryNeedle),
    ContainsCaseInsensitive(QueryNeedle),
    EndsWithCaseInsensitive(QueryNeedle),
    EqualsCaseInsensitive(QueryNeedle),
}

impl QueryRule {
    #[must_use]
    pub fn parse(raw_term: &str) -> Option<Self> {
        if raw_term.is_empty() {
            return None;
        }

        if let Some(inner) = raw_term.strip_prefix('<') {
            if let Some(exact) = inner.strip_suffix('>') {
                if exact.is_empty() {
                    return None;
                }
                return Some(Self::EqualsCaseInsensitive(QueryNeedle::new(exact)));
            }

            if inner.is_empty() {
                return None;
            }
            return Some(Self::PrefixCaseInsensitive(QueryNeedle::new(inner)));
        }

        if let Some(suffix) = raw_term.strip_suffix('>') {
            if suffix.is_empty() {
                return None;
            }
            Some(Self::EndsWithCaseInsensitive(QueryNeedle::new(suffix)))
        } else {
            Some(Self::ContainsCaseInsensitive(QueryNeedle::new(raw_term)))
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::PrefixCaseInsensitive(needle)
            | Self::ContainsCaseInsensitive(needle)
            | Self::EndsWithCaseInsensitive(needle)
            | Self::EqualsCaseInsensitive(needle) => needle.is_empty(),
        }
    }

    #[must_use]
    pub fn matches(&self, haystack: &str) -> bool {
        self.matches_preprocessed(haystack, None)
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        match self {
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

impl Display for QueryRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrefixCaseInsensitive(needle) => write!(f, "<{}", needle.normalized_str()),
            Self::ContainsCaseInsensitive(needle) => write!(f, "{}", needle.normalized_str()),
            Self::EndsWithCaseInsensitive(needle) => write!(f, "{}>", needle.normalized_str()),
            Self::EqualsCaseInsensitive(needle) => write!(f, "<{}>", needle.normalized_str()),
        }
    }
}
