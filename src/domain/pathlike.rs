use facet::Facet;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;

/// Facet-safe path-like wrapper around String for paths in query results.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[facet(opaque, proxy = String)]
#[repr(transparent)]
pub struct Pathlike(String);

impl Pathlike {
    #[must_use]
    pub fn new(path: String) -> Self {
        Self(path)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    #[must_use]
    pub fn display(&self) -> impl std::fmt::Display + '_ {
        self.as_path().display()
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for Pathlike {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for Pathlike {
    fn from(value: &str) -> Self {
        Self::new(String::from(value))
    }
}

impl From<Pathlike> for String {
    fn from(value: Pathlike) -> Self {
        value.0
    }
}

impl From<&Pathlike> for String {
    fn from(value: &Pathlike) -> Self {
        value.0.clone()
    }
}

impl From<Pathlike> for PathBuf {
    fn from(value: Pathlike) -> Self {
        Self::from(value.0)
    }
}

impl Deref for Pathlike {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.as_path()
    }
}

impl AsRef<Path> for Pathlike {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<str> for Pathlike {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for Pathlike {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl std::fmt::Display for Pathlike {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::Pathlike;

    #[test]
    fn wraps_index_path_string_without_pathbuf_conversion() {
        let path = Pathlike::from(String::from(r"C:\music\track.flac"));

        assert_eq!(path.as_str(), r"C:\music\track.flac");
        assert_eq!(path.as_path(), std::path::Path::new(r"C:\music\track.flac"));
        assert_eq!(path.clone().into_string(), r"C:\music\track.flac");
    }
}
