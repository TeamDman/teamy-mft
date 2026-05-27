use facet::Facet;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[facet(opaque, proxy = String)]
#[repr(transparent)]
pub struct QueryResultPath(String);

impl QueryResultPath {
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

impl From<String> for QueryResultPath {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<QueryResultPath> for String {
    fn from(value: QueryResultPath) -> Self {
        value.0
    }
}

impl From<&QueryResultPath> for String {
    fn from(value: &QueryResultPath) -> Self {
        value.0.clone()
    }
}

impl From<QueryResultPath> for PathBuf {
    fn from(value: QueryResultPath) -> Self {
        Self::from(value.0)
    }
}

impl Deref for QueryResultPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<Path> for QueryResultPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<str> for QueryResultPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for QueryResultPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::QueryResultPath;

    #[test]
    fn wraps_index_path_string_without_pathbuf_conversion() {
        let path = QueryResultPath::from(String::from(r"C:\music\track.flac"));

        assert_eq!(path.as_str(), r"C:\music\track.flac");
        assert_eq!(path.as_path(), std::path::Path::new(r"C:\music\track.flac"));
        assert_eq!(path.clone().into_string(), r"C:\music\track.flac");
    }
}
