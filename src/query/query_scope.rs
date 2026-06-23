use eyre::Context;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct QueryScope {
    pub(crate) root: PathBuf,
    pub(crate) include_descendants: bool,
}

impl QueryScope {
    #[must_use]
    pub fn matches_path(&self, path: &Path) -> bool {
        path_matches_scope(path, self)
    }

    #[must_use]
    pub(crate) fn normalized_components(&self) -> Vec<String> {
        lowercase_path_components(&self.root)
    }
}

pub(crate) fn resolve_query_scopes(scopes: &[String]) -> eyre::Result<Vec<QueryScope>> {
    scopes
        .iter()
        .map(String::as_str)
        .map(|scope| resolve_query_scope(Some(scope)))
        .map(|scope| scope.map(|scope| scope.expect("single scope should resolve to Some")))
        .collect()
}

pub(crate) fn resolve_query_scope(scope: Option<&str>) -> eyre::Result<Option<QueryScope>> {
    let Some(scope) = scope else {
        return Ok(None);
    };

    let root = dunce::canonicalize(scope)
        .wrap_err_with(|| format!("Failed resolving query scope from {scope}"))?;

    Ok(Some(QueryScope {
        include_descendants: root.is_dir(),
        root,
    }))
}

pub(crate) fn lowercase_path_components(path: &Path) -> Vec<String> {
    let path = path.as_os_str().to_string_lossy().replace('/', "\\");
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map_or_else(|| path.clone(), |rest| format!(r"\\{rest}"));
    let path = path
        .strip_prefix(r"\\?\")
        .map_or_else(|| path.clone(), ToString::to_string);

    path.split('\\')
        .filter(|component| !component.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn path_matches_scope(path: &Path, scope: &QueryScope) -> bool {
    if cfg!(windows) {
        let path_components = lowercase_path_components(path);
        let scope_components = lowercase_path_components(&scope.root);

        return if scope.include_descendants {
            path_components.starts_with(&scope_components)
        } else {
            path_components == scope_components
        };
    }

    if scope.include_descendants {
        path.starts_with(&scope.root)
    } else {
        path == scope.root
    }
}

#[cfg(test)]
fn should_include_scope(path: &str, scope: Option<&QueryScope>) -> bool {
    // cli[impl command.query.scope-filter]
    let Some(scope) = scope else {
        return true;
    };

    path_matches_scope(Path::new(path), scope)
}

#[cfg(test)]
mod tests {
    use super::resolve_query_scope;
    use super::should_include_scope;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::OnceLock;

    fn current_dir_lock() -> &'static Mutex<()> {
        static CURRENT_DIR_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        CURRENT_DIR_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct CurrentDirRestore(PathBuf);

    impl Drop for CurrentDirRestore {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    #[cfg(windows)]
    fn verbatim_path(path: &Path) -> String {
        format!(r"\\?\{}", path.display())
    }

    #[cfg(windows)]
    fn has_verbatim_prefix(path: &Path) -> bool {
        path.to_string_lossy().starts_with(r"\\?\")
    }

    #[test]
    fn directory_matches_descendants_but_not_sibling_prefixes() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_dir = temp_dir.path().join("repo");
        let nested_file = scope_dir.join("music").join("song.mp3");
        let sibling_file = temp_dir.path().join("repo2").join("song.mp3");

        std::fs::create_dir_all(
            nested_file
                .parent()
                .expect("nested file should have parent"),
        )?;
        std::fs::create_dir_all(
            sibling_file
                .parent()
                .expect("sibling file should have parent"),
        )?;
        std::fs::write(&nested_file, [])?;
        std::fs::write(&sibling_file, [])?;

        let scope = resolve_query_scope(Some(&scope_dir.to_string_lossy()))?
            .expect("directory scope should resolve");
        let nested_file = dunce::canonicalize(&nested_file)?;
        let sibling_file = dunce::canonicalize(&sibling_file)?;

        assert!(should_include_scope(
            &nested_file.to_string_lossy(),
            Some(&scope)
        ));
        assert!(!should_include_scope(
            &sibling_file.to_string_lossy(),
            Some(&scope)
        ));

        Ok(())
    }

    #[test]
    fn file_matches_only_exact_path() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_file = temp_dir.path().join("track.flac");
        let other_file = temp_dir.path().join("track.flac.bak");

        std::fs::write(&scope_file, [])?;
        std::fs::write(&other_file, [])?;

        let scope = resolve_query_scope(Some(&scope_file.to_string_lossy()))?
            .expect("file scope should resolve");
        let scope_file = dunce::canonicalize(&scope_file)?;
        let other_file = dunce::canonicalize(&other_file)?;

        assert!(should_include_scope(
            &scope_file.to_string_lossy(),
            Some(&scope)
        ));
        assert!(!should_include_scope(
            &other_file.to_string_lossy(),
            Some(&scope)
        ));

        Ok(())
    }

    #[test]
    fn dot_resolves_against_current_working_directory() -> eyre::Result<()> {
        let _lock = current_dir_lock()
            .lock()
            .expect("current dir test lock should not be poisoned");
        let temp_dir = tempfile::tempdir()?;
        let original_dir = std::env::current_dir()?;
        let _restore = CurrentDirRestore(original_dir);

        std::env::set_current_dir(temp_dir.path())?;

        let scope = resolve_query_scope(Some("."))?.expect("dot scope should resolve");

        assert_eq!(scope.root, dunce::canonicalize(temp_dir.path())?);
        assert!(scope.include_descendants);

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn resolve_query_scope_removes_verbatim_prefix_for_directories() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(&scope_dir)?;

        let scope = resolve_query_scope(Some(&verbatim_path(&scope_dir)))?
            .expect("directory scope should resolve");

        assert_eq!(scope.root, dunce::canonicalize(&scope_dir)?);
        assert!(!has_verbatim_prefix(&scope.root));

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn resolve_query_scope_removes_verbatim_prefix_for_files() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_file = temp_dir.path().join("track.flac");
        std::fs::write(&scope_file, [])?;

        let scope = resolve_query_scope(Some(&verbatim_path(&scope_file)))?
            .expect("file scope should resolve");

        assert_eq!(scope.root, dunce::canonicalize(&scope_file)?);
        assert!(!has_verbatim_prefix(&scope.root));

        Ok(())
    }
}
