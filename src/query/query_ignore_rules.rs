use crate::query::QueryPlan;
use crate::query::matching_row_indices_for_rule;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use eyre::Context;
use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;
use rayon::prelude::*;
use std::path::Path;
use std::path::PathBuf;
use tracing::debug;
use tracing::info_span;
use tracing::warn;

pub const IGNORE_FILE_EXTENSION: &str = ".teamymftignore";
pub const SYNCED_IGNORE_FILE_NAME: &str = "teamy-mft-sync.teamymftignore";

pub struct QueryIgnoreRules {
    matcher: Gitignore,
    files: Vec<DiscoveredIgnoreFile>,
}

pub struct DiscoveredIgnoreFile {
    pub drive_letter: char,
    pub path: PathBuf,
    pub rules: Vec<IgnoreRuleLine>,
    matcher: Gitignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRuleLine {
    pub line_number: usize,
    pub pattern: String,
}

impl QueryIgnoreRules {
    /// Build an empty ignore set that matches nothing.
    ///
    /// # Panics
    ///
    /// Panics if building an empty ignore matcher unexpectedly fails.
    #[must_use]
    pub fn empty() -> Self {
        let builder = GitignoreBuilder::new(".");
        let matcher = builder
            .build()
            .expect("empty ignore matcher should always build");
        Self {
            matcher,
            files: Vec::new(),
        }
    }

    /// Discover `.teamymftignore` files from the cached search indexes for the given drives.
    ///
    /// # Errors
    ///
    /// Returns an error if a search index cannot be opened or parsed, if an ignore file cannot
    /// be parsed as gitignore syntax, or if a discovered ignore file cannot be read.
    pub fn discover_for_drive_letters(
        drive_letters: &[char],
        sync_dir: &Path,
    ) -> eyre::Result<Self> {
        let discovered = {
            let _span = info_span!("discover_query_ignore_files").entered();
            let results: Vec<eyre::Result<Vec<DiscoveredIgnoreFile>>> = drive_letters
                .par_iter()
                .map(|drive_letter| discover_ignore_files_for_drive(*drive_letter, sync_dir))
                .collect();

            let mut discovered = Vec::new();
            for result in results {
                discovered.extend(result?);
            }
            discovered
        };

        Self::from_discovered_files(discovered)
    }

    /// Construct a shared ignore set from already discovered files.
    ///
    /// # Errors
    ///
    /// Returns an error if the combined gitignore matcher cannot be built.
    pub fn from_discovered_files(mut files: Vec<DiscoveredIgnoreFile>) -> eyre::Result<Self> {
        files.sort_by(|left, right| left.path.cmp(&right.path));

        let mut builder = GitignoreBuilder::new(".");
        for file in &files {
            for rule in &file.rules {
                builder.add_line(None, &rule.pattern).wrap_err_with(|| {
                    format!(
                        "Failed parsing ignore rule {}:{}",
                        file.path.display(),
                        rule.line_number
                    )
                })?;
            }
        }
        let matcher = builder
            .build()
            .wrap_err("Failed building combined ignore matcher")?;

        Ok(Self { matcher, files })
    }

    #[must_use]
    pub fn files(&self) -> &[DiscoveredIgnoreFile] {
        &self.files
    }

    #[must_use]
    pub fn merged_rule_lines(&self) -> Vec<String> {
        let mut merged = Vec::new();

        for file in &self.files {
            for rule in &file.rules {
                if merged.iter().any(|existing| existing == &rule.pattern) {
                    continue;
                }
                merged.push(rule.pattern.clone());
            }
        }

        merged
    }

    #[must_use]
    pub fn is_ignored_path(&self, path: &Path) -> bool {
        let normalized_path = normalize_candidate_path(path);
        self.matcher
            .matched_path_or_any_parents(&normalized_path, path.is_dir())
            .is_ignore()
    }

    #[must_use]
    pub fn matching_files_for_path(&self, path: &Path) -> Vec<&DiscoveredIgnoreFile> {
        let normalized_path = normalize_candidate_path(path);
        self.files
            .iter()
            .filter(|file| {
                file.matcher
                    .matched_path_or_any_parents(&normalized_path, path.is_dir())
                    .is_ignore()
            })
            .collect()
    }
}

fn discover_ignore_files_for_drive(
    drive_letter: char,
    sync_dir: &Path,
) -> eyre::Result<Vec<DiscoveredIgnoreFile>> {
    let index_path = sync_dir.join(format!("{drive_letter}.mft_search_index"));
    if !index_path.is_file() {
        return Ok(Vec::new());
    }

    let mapped = MappedSearchIndex::open(&index_path).wrap_err_with(|| {
        format!(
            "Failed loading search index for drive {} from {}",
            drive_letter,
            index_path.display()
        )
    })?;
    let parsed_index = SearchIndexBytes::new(mapped.bytes())
        .parse_trusted_for_query()
        .wrap_err_with(|| {
            format!(
                "Failed preparing search index rows for drive {} from {}",
                drive_letter,
                index_path.display()
            )
        })?;
    let ignore_query = QueryPlan::parse_inputs(&[IGNORE_FILE_EXTENSION.to_owned()])?;
    let ignore_rows = ignore_query
        .matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed_index, rule))
        .wrap_err_with(|| {
            format!(
                "Failed matching ignore rows for drive {} from {}",
                drive_letter,
                index_path.display()
            )
        })?;

    let mut files = Vec::new();
    for row_index in ignore_rows {
        let row = parsed_index
            .row_view(row_index as usize)
            .wrap_err_with(|| {
                format!(
                    "Failed reading ignore row {row_index} from {}",
                    index_path.display()
                )
            })?;
        if row.has_deleted_entries {
            continue;
        }
        let path = PathBuf::from(row.path());
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(IGNORE_FILE_EXTENSION))
        {
            continue;
        }
        let Some(file) = load_ignore_file(drive_letter, &path)? else {
            continue;
        };
        files.push(file);
    }

    Ok(files)
}

fn load_ignore_file(drive_letter: char, path: &Path) -> eyre::Result<Option<DiscoveredIgnoreFile>> {
    if !path.is_file() {
        debug!(path = %path.display(), "Skipping ignore file that no longer exists");
        return Ok(None);
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            warn!(
                drive = %drive_letter,
                path = %path.display(),
                error = %error,
                "Skipping unreadable ignore file during query discovery"
            );
            return Ok(None);
        }
        Err(error) => {
            return Err(error)
                .wrap_err_with(|| format!("Failed reading ignore file {}", path.display()));
        }
    };
    let rules = parse_ignore_rule_lines(&contents);

    let mut builder = GitignoreBuilder::new(".");
    for rule in &rules {
        builder.add_line(None, &rule.pattern).wrap_err_with(|| {
            format!(
                "Failed parsing ignore rule {}:{}",
                path.display(),
                rule.line_number
            )
        })?;
    }
    let matcher = builder.build().wrap_err_with(|| {
        format!(
            "Failed parsing ignore file {} as gitignore syntax",
            path.display()
        )
    })?;

    Ok(Some(DiscoveredIgnoreFile {
        drive_letter,
        path: path.to_path_buf(),
        rules,
        matcher,
    }))
}

fn parse_ignore_rule_lines(contents: &str) -> Vec<IgnoreRuleLine> {
    contents
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            if line.trim().is_empty() || line.starts_with('#') {
                return None;
            }

            Some(IgnoreRuleLine {
                line_number: index + 1,
                pattern: line.to_owned(),
            })
        })
        .collect()
}

fn normalize_candidate_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::CurDir => {}
            std::path::Component::ParentDir => normalized.push(".."),
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

impl std::fmt::Debug for QueryIgnoreRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryIgnoreRules")
            .field("files", &self.files)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DiscoveredIgnoreFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredIgnoreFile")
            .field("drive_letter", &self.drive_letter)
            .field("path", &self.path)
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::IGNORE_FILE_EXTENSION;
    use super::QueryIgnoreRules;
    use super::SYNCED_IGNORE_FILE_NAME;
    use crate::query::QueryPlan;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;

    #[test]
    fn merged_rule_lines_preserve_first_seen_order_while_deduping() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path_a = temp_dir.path().join(format!("a{IGNORE_FILE_EXTENSION}"));
        let path_b = temp_dir.path().join(format!("b{IGNORE_FILE_EXTENSION}"));
        std::fs::write(&path_a, "FirstName\nSecondName\n").expect("write path_a");
        std::fs::write(&path_b, "SecondName\nThirdName\n").expect("write path_b");

        let rules = QueryIgnoreRules::from_discovered_files(vec![
            super::load_ignore_file('C', &path_a)
                .expect("load path_a")
                .expect("path_a exists"),
            super::load_ignore_file('D', &path_b)
                .expect("load path_b")
                .expect("path_b exists"),
        ])
        .expect("build ignore set");

        assert_eq!(
            rules.merged_rule_lines(),
            vec![
                String::from("FirstName"),
                String::from("SecondName"),
                String::from("ThirdName")
            ]
        );
    }

    #[test]
    fn ignores_paths_using_gitignore_semantics() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let ignore_path = temp_dir
            .path()
            .join(format!("sample{IGNORE_FILE_EXTENSION}"));
        let nested_dir = temp_dir.path().join("private");
        let nested_file = nested_dir.join("notes.txt");
        std::fs::create_dir_all(&nested_dir).expect("create nested dir");
        std::fs::write(&nested_file, []).expect("write nested file");
        std::fs::write(&ignore_path, "private/\n").expect("write ignore file");

        let rules = QueryIgnoreRules::from_discovered_files(vec![
            super::load_ignore_file('C', &ignore_path)
                .expect("load ignore file")
                .expect("ignore file exists"),
        ])
        .expect("build ignore set");

        assert!(rules.is_ignored_path(&nested_file));
        assert_eq!(rules.matching_files_for_path(&nested_file).len(), 1);
    }

    #[test]
    fn synced_ignore_file_name_uses_expected_extension() {
        assert!(SYNCED_IGNORE_FILE_NAME.ends_with(IGNORE_FILE_EXTENSION));
    }

    #[test]
    fn ignore_file_suffix_query_matches_teamy_ignore_paths() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\repo\\2026-05-23.teamymftignore"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\.gitignore"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        let parsed = crate::search_index::search_index_bytes::SearchIndexBytes::new(bytes)
            .parse_trusted_for_query()?;
        let plan = QueryPlan::parse_inputs(&[IGNORE_FILE_EXTENSION.to_owned()])?;

        let indices = plan.matching_row_indices(&|rule| {
            crate::query::matching_row_indices_for_rule(&parsed, rule)
        })?;

        assert_eq!(indices, vec![0]);
        Ok(())
    }
}
