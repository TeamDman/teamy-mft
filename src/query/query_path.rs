use facet::Facet;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct QueryPath(String);

impl QueryPath {
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

impl From<String> for QueryPath {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<QueryPath> for String {
    fn from(value: QueryPath) -> Self {
        value.0
    }
}

impl From<QueryPath> for PathBuf {
    fn from(value: QueryPath) -> Self {
        Self::from(value.0)
    }
}

impl Deref for QueryPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<Path> for QueryPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<str> for QueryPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for QueryPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "facet proxy conversion ABI requires Result even though QueryPath cannot fail"
)]
unsafe fn query_path_proxy_convert_out(
    target_ptr: facet::PtrConst,
    proxy_ptr: facet::PtrUninit,
) -> Result<facet::PtrMut, String> {
    // SAFETY: `target_ptr` points at a valid `QueryPath` and `proxy_ptr` points at
    // facet-managed storage for a `String` proxy with the correct layout.
    unsafe {
        let path = target_ptr.get::<QueryPath>();
        #[allow(
            clippy::cast_ptr_alignment,
            reason = "facet allocates proxy storage with the alignment required by the proxy type"
        )]
        let proxy_mut = proxy_ptr.as_mut_byte_ptr().cast::<String>();
        proxy_mut.write(path.0.clone());
        Ok(facet::PtrMut::new(proxy_mut.cast::<u8>()))
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "facet proxy conversion ABI requires Result even though QueryPath cannot fail"
)]
unsafe fn query_path_proxy_convert_in(
    proxy_ptr: facet::PtrConst,
    target_ptr: facet::PtrUninit,
) -> Result<facet::PtrMut, String> {
    // SAFETY: `proxy_ptr` points at a valid `String` proxy and `target_ptr` points at
    // facet-managed storage for a `QueryPath` destination with the correct layout.
    unsafe {
        let path = proxy_ptr.read::<String>();
        #[allow(
            clippy::cast_ptr_alignment,
            reason = "facet allocates target storage with the alignment required by the target type"
        )]
        let target_mut = target_ptr.as_mut_byte_ptr().cast::<QueryPath>();
        target_mut.write(QueryPath(path));
        Ok(facet::PtrMut::new(target_mut.cast::<u8>()))
    }
}

const QUERY_PATH_PROXY: facet::ProxyDef = facet::ProxyDef {
    shape: <String as Facet>::SHAPE,
    convert_in: query_path_proxy_convert_in,
    convert_out: query_path_proxy_convert_out,
};

// SAFETY: `QueryPath` is serialized through an owned `String` proxy, preserving
// the index-produced path representation without converting through `PathBuf`.
unsafe impl Facet<'_> for QueryPath {
    const SHAPE: &'static facet::Shape = &const {
        facet::ShapeBuilder::for_sized::<QueryPath>("QueryPath")
            .module_path("teamy_mft::query")
            .ty(facet::Type::User(facet::UserType::Opaque))
            .def(facet::Def::Scalar)
            .proxy(&QUERY_PATH_PROXY)
            .build()
    };
}

#[cfg(test)]
mod tests {
    use super::QueryPath;

    #[test]
    fn wraps_index_path_string_without_pathbuf_conversion() {
        let path = QueryPath::from(String::from(r"C:\music\track.flac"));

        assert_eq!(path.as_str(), r"C:\music\track.flac");
        assert_eq!(path.as_path(), std::path::Path::new(r"C:\music\track.flac"));
        assert_eq!(path.clone().into_string(), r"C:\music\track.flac");
    }
}
