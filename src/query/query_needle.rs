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
    pub fn normalized_str(&self) -> &str {
        match self {
            // SAFETY: `AsciiLower` is constructed only from ASCII bytes,
            // which are always valid UTF-8.
            Self::AsciiLower(needle) => unsafe { std::str::from_utf8_unchecked(needle) },
            Self::UnicodeLower(needle) => needle,
        }
    }

    #[must_use]
    pub fn matches_contains(&self, haystack: &str) -> bool {
        self.matches_contains_preprocessed(haystack, None)
    }

    #[must_use]
    pub fn matches_contains_preprocessed(
        &self,
        haystack: &str,
        normalized_haystack: Option<&str>,
    ) -> bool {
        match self {
            Self::AsciiLower(needle) => normalized_haystack.map_or_else(
                || contains_case_insensitive_ascii(haystack, needle),
                |normalized| contains_ascii_in_normalized_haystack(normalized, needle),
            ),
            Self::UnicodeLower(needle) => normalized_haystack.map_or_else(
                || haystack.to_lowercase().contains(needle),
                |normalized| normalized.contains(needle),
            ),
        }
    }

    #[must_use]
    pub fn matches_suffix(&self, haystack: &str) -> bool {
        self.matches_suffix_preprocessed(haystack, None)
    }

    #[must_use]
    pub fn matches_suffix_preprocessed(
        &self,
        haystack: &str,
        normalized_haystack: Option<&str>,
    ) -> bool {
        match self {
            Self::AsciiLower(needle) => normalized_haystack.map_or_else(
                || ends_with_case_insensitive_ascii(haystack, needle),
                |normalized| ends_with_ascii_in_normalized_haystack(normalized, needle),
            ),
            Self::UnicodeLower(needle) => normalized_haystack.map_or_else(
                || haystack.to_lowercase().ends_with(needle),
                |normalized| normalized.ends_with(needle),
            ),
        }
    }
}

fn contains_ascii_in_normalized_haystack(haystack: &str, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack = haystack.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }

    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn ends_with_ascii_in_normalized_haystack(haystack: &str, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack = haystack.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }

    &haystack[haystack.len() - needle.len()..] == needle
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
