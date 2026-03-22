#[derive(Debug, Clone)]
pub enum QueryNeedle {
    AsciiLower(Vec<u8>),
    UnicodeLower(String),
}

impl QueryNeedle {
    #[must_use]
    pub fn new(value: &str) -> Self {
        if value.is_ascii() {
            Self::AsciiLower(
                value
                    .bytes()
                    .map(|byte| byte.to_ascii_lowercase())
                    .collect(),
            )
        } else {
            Self::UnicodeLower(value.to_lowercase())
        }
    }

    #[must_use]
    pub fn matches_contains(&self, haystack: &str) -> bool {
        match self {
            Self::AsciiLower(needle) => contains_case_insensitive_ascii(haystack, needle),
            Self::UnicodeLower(needle) => haystack.to_lowercase().contains(needle),
        }
    }

    #[must_use]
    pub fn matches_suffix(&self, haystack: &str) -> bool {
        match self {
            Self::AsciiLower(needle) => ends_with_case_insensitive_ascii(haystack, needle),
            Self::UnicodeLower(needle) => haystack.to_lowercase().ends_with(needle),
        }
    }
}

fn contains_case_insensitive_ascii(haystack: &str, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack = haystack.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }

    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle.iter())
            .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
    })
}

fn ends_with_case_insensitive_ascii(haystack: &str, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack = haystack.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }

    haystack[haystack.len() - needle.len()..]
        .iter()
        .zip(needle.iter())
        .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
}
