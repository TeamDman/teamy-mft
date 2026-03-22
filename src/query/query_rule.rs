use crate::query::QueryNeedle;
use crate::query::query_needle::QUERY_TRIGRAM_LEN;

#[derive(Debug, Clone)]
pub enum QueryRule {
    ContainsCaseInsensitive(QueryNeedle),
    EndsWithCaseInsensitive(QueryNeedle),
}

impl QueryRule {
    #[must_use]
    pub fn parse(raw_term: &str) -> Option<Self> {
        if raw_term.is_empty() {
            return None;
        }

        if let Some(suffix) = raw_term.strip_suffix('$') {
            if suffix.is_empty() {
                return None;
            }
            Some(Self::EndsWithCaseInsensitive(QueryNeedle::new(suffix)))
        } else {
            Some(Self::ContainsCaseInsensitive(QueryNeedle::new(raw_term)))
        }
    }

    #[must_use]
    pub fn matches(&self, haystack: &str) -> bool {
        self.matches_preprocessed(haystack, None)
    }

    #[must_use]
    pub fn matches_preprocessed(&self, haystack: &str, normalized_haystack: Option<&str>) -> bool {
        match self {
            Self::ContainsCaseInsensitive(needle) => {
                needle.matches_contains_preprocessed(haystack, normalized_haystack)
            }
            Self::EndsWithCaseInsensitive(needle) => {
                needle.matches_suffix_preprocessed(haystack, normalized_haystack)
            }
        }
    }

    #[must_use]
    pub fn matches_normalized(&self, normalized_haystack: &str) -> bool {
        self.matches_preprocessed(normalized_haystack, Some(normalized_haystack))
    }

    #[must_use]
    pub fn normalized_extension_suffix(&self) -> Option<&str> {
        match self {
            Self::EndsWithCaseInsensitive(needle) => {
                let suffix = needle.normalized_str();
                (suffix.starts_with('.') && suffix.len() > 1).then_some(suffix)
            }
            Self::ContainsCaseInsensitive(_) => None,
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
            Self::ContainsCaseInsensitive(_) | Self::EndsWithCaseInsensitive(_) => None,
        }
    }
}
