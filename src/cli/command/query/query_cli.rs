use crate::query::IndexedPathRow;
use crate::query::QueryPlan;
use crate::query::QueryRule;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::ParsedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use color_eyre::owo_colors::OwoColorize;
use eyre::Context;
use facet::Facet;
use figue::{self as args};
use rayon::prelude::*;
use std::io::IsTerminal;
use std::path::Path;
use std::path::PathBuf;
use teamy_windows::storage::DriveLetterPattern;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use tracing::instrument;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default)]
#[facet(rename_all = "kebab-case")]
pub struct QueryArgs {
    /// Fast query groups. Each positional argument is `OR`ed; whitespace-delimited terms within one argument are `AND`ed.
    #[facet(args::positional, default)]
    pub query: Vec<String>,
    /// Restrict results to this path. Directories include descendants; files match exactly.
    #[facet(args::named, default)]
    pub r#in: Option<String>,
    /// Drive letter pattern to match drives whose cached MFTs will be queried (e.g., "*", "C", "CD", "C,D")
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Maximum number of results to show
    #[facet(args::named, default)]
    pub limit: usize,
    /// Include paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub include_deleted: bool,
    /// Show only paths that contain one or more deleted MFT entries
    #[facet(args::named, default)]
    pub only_deleted: bool,
    /// Output density mode
    #[facet(args::named, default)]
    pub density: QueryDensity,
}

#[derive(Default, Facet, Arbitrary, Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum QueryDensity {
    #[default]
    Auto,
    Lines,
    Columns,
}

#[derive(Debug, Default)]
struct DriveQueryResult {
    loaded_rows: usize,
    matched_rows: Vec<IndexedPathRow>,
}

#[derive(Debug, Clone)]
struct QueryScope {
    root: PathBuf,
    include_descendants: bool,
}

fn render_indexed_path(row: &IndexedPathRow, colorize: bool) -> String {
    if !colorize {
        return row.path.clone();
    }
    if row.has_deleted_entries {
        row.path.red().to_string()
    } else {
        row.path.green().to_string()
    }
}

fn string_display_width(value: &str) -> usize {
    value.chars().count()
}

fn detect_terminal_columns() -> Option<usize> {
    crossterm::terminal::size()
        .ok()
        .map(|(columns, _)| usize::from(columns))
        .filter(|value| *value > 0)
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
}

fn print_results_lines(results: &[IndexedPathRow], colorize: bool) {
    for row in results {
        println!("{}", render_indexed_path(row, colorize));
    }
}

fn print_results_columns(results: &[IndexedPathRow], colorize: bool) {
    if results.is_empty() {
        return;
    }

    let gap = 2usize;
    let max_width = results
        .iter()
        .map(|row| string_display_width(&row.path))
        .max()
        .unwrap_or(1)
        .max(1);
    let terminal_columns = detect_terminal_columns().unwrap_or(120usize);

    let column_count = ((terminal_columns + gap) / (max_width + gap)).max(1);
    let row_count = results.len().div_ceil(column_count);

    for row_index in 0..row_count {
        let mut line = String::new();

        for column_index in 0..column_count {
            let index = row_index + column_index * row_count;
            if index >= results.len() {
                continue;
            }

            let row = &results[index];
            line.push_str(&render_indexed_path(row, colorize));

            if column_index + 1 < column_count {
                let pad = (max_width + gap).saturating_sub(string_display_width(&row.path));
                line.push_str(&" ".repeat(pad));
            }
        }

        println!("{line}");
    }
}

fn should_include_indexed_row(
    include_deleted: bool,
    only_deleted: bool,
    has_deleted_entries: bool,
) -> bool {
    if only_deleted {
        return has_deleted_entries;
    }

    include_deleted || !has_deleted_entries
}

fn resolve_query_scope(scope: Option<&str>) -> eyre::Result<Option<QueryScope>> {
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

fn lowercase_path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
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

fn should_include_scope(path: &str, scope: Option<&QueryScope>) -> bool {
    let Some(scope) = scope else {
        return true;
    };

    path_matches_scope(Path::new(path), scope)
}

fn matching_row_indices_for_rule(
    parsed_index: &ParsedSearchIndex<'_>,
    rule: &QueryRule,
) -> eyre::Result<Vec<u32>> {
    if let Some(normalized_suffix) = rule.normalized_extension_suffix() {
        return Ok(match parsed_index.extension_postings(normalized_suffix)? {
            Some(iter) => iter.collect(),
            None => Vec::new(),
        });
    }

    if rule.matches_only_terminal_segment() {
        return terminal_matching_row_indices_for_rule(parsed_index, rule);
    }

    let matching_segment_ids = matching_segment_ids_for_rule(parsed_index, rule)?;

    let mut row_indices = {
        let _span = info_span!("collect_matching_segment_postings").entered();
        let mut row_indices = Vec::new();

        for segment_id in matching_segment_ids {
            row_indices.extend(parsed_index.postings(segment_id)?);
        }

        row_indices
    };

    info_span!("normalize_matching_row_indices").in_scope(|| {
        row_indices.sort_unstable();
        row_indices.dedup();
    });

    Ok(row_indices)
}

fn terminal_matching_row_indices_for_rule(
    parsed_index: &ParsedSearchIndex<'_>,
    rule: &QueryRule,
) -> eyre::Result<Vec<u32>> {
    info_span!("match_query_rule_against_terminal_segments").in_scope(|| {
        let mut row_indices = Vec::new();

        for (row_index, row) in parsed_index.row_views().enumerate() {
            let row = row?;
            let Some(terminal_segment) = row.segment_views().next() else {
                continue;
            };

            if !rule.matches_normalized(terminal_segment.normalized) {
                continue;
            }

            let row_index = u32::try_from(row_index).wrap_err_with(|| {
                format!("Row index {row_index} does not fit into u32 for query results")
            })?;
            row_indices.push(row_index);
        }

        Ok(row_indices)
    })
}

fn matching_segment_ids_for_rule(
    parsed_index: &ParsedSearchIndex<'_>,
    rule: &QueryRule,
) -> eyre::Result<Vec<u32>> {
    if let Some(trigrams) = rule.normalized_contains_trigrams() {
        let candidate_segment_ids = {
            let _span = info_span!("collect_trigram_segment_candidates").entered();
            let mut candidate_segment_ids: Option<Vec<u32>> = None;

            for trigram in trigrams {
                let Some(iter) = parsed_index.trigram_postings(trigram)? else {
                    return Ok(Vec::new());
                };

                let mut trigram_segment_ids = iter.collect::<Vec<_>>();
                trigram_segment_ids.sort_unstable();
                trigram_segment_ids.dedup();

                candidate_segment_ids = Some(match candidate_segment_ids.take() {
                    Some(existing_segment_ids) => {
                        intersect_sorted_ids(&existing_segment_ids, &trigram_segment_ids)
                    }
                    None => trigram_segment_ids,
                });

                if candidate_segment_ids
                    .as_ref()
                    .is_some_and(std::vec::Vec::is_empty)
                {
                    return Ok(Vec::new());
                }
            }

            candidate_segment_ids.unwrap_or_default()
        };

        return info_span!("filter_trigram_segment_candidates").in_scope(|| {
            let mut matching_segment_ids = Vec::with_capacity(candidate_segment_ids.len());

            for segment_id in candidate_segment_ids {
                let segment = parsed_index.segment(segment_id)?;
                if rule.matches_normalized(segment.normalized) {
                    matching_segment_ids.push(segment_id);
                }
            }

            Ok(matching_segment_ids)
        });
    }

    info_span!("match_query_rule_against_segments").in_scope(|| {
        let mut matching_segment_ids = Vec::new();

        for (segment_id, segment) in parsed_index.segments().iter().enumerate() {
            if !rule.matches_normalized(segment.normalized) {
                continue;
            }

            let segment_id = u32::try_from(segment_id).wrap_err_with(|| {
                format!("Segment id {segment_id} does not fit into u32 for postings lookup")
            })?;
            matching_segment_ids.push(segment_id);
        }

        Ok(matching_segment_ids)
    })
}

fn intersect_sorted_ids(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut left_index = 0;
    let mut right_index = 0;
    let mut intersection = Vec::with_capacity(left.len().min(right.len()));

    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => {
                intersection.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }

    intersection
}

fn load_and_query_drive_search_index(
    drive_letter: char,
    sync_dir: &Path,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<DriveQueryResult> {
    let _span = info_span!("load_drive_search_index").entered();
    let index_path = sync_dir.join(format!("{drive_letter}.mft_search_index"));

    {
        let _span = info_span!("validate_search_index_file").entered();
        if !index_path.is_file() {
            eyre::bail!(
                "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
                index_path.display(),
                drive_letter
            );
        }
    }

    let mapped = {
        let _span = info_span!("map_search_index_file").entered();
        MappedSearchIndex::open(&index_path).wrap_err_with(|| {
            format!(
                "Failed loading search index for drive {} from {}",
                drive_letter,
                index_path.display()
            )
        })?
    };

    let parsed_index = {
        let _span = info_span!("parse_search_index_for_query").entered();
        SearchIndexBytes::new(mapped.bytes())
            .parse_trusted_for_query()
            .wrap_err_with(|| {
                format!(
                    "Failed preparing search index rows for drive {} from {}",
                    drive_letter,
                    index_path.display()
                )
            })?
    };

    let loaded_rows = parsed_index.row_count();
    let matched_row_indices = {
        let _span = info_span!("match_search_index_postings").entered();
        query_plan
            .matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed_index, rule))
            .wrap_err_with(|| {
                format!(
                    "Failed matching search index rows for drive {} from {}",
                    drive_letter,
                    index_path.display()
                )
            })?
    };
    let matched_rows = {
        let _span = info_span!("materialize_matched_index_rows").entered();
        let mut matched_rows = Vec::with_capacity(matched_row_indices.len());

        for row_index in matched_row_indices {
            let row = parsed_index
                .row_view(row_index as usize)
                .wrap_err_with(|| {
                    format!(
                        "Failed materializing search index row {} for drive {} from {}",
                        row_index,
                        drive_letter,
                        index_path.display()
                    )
                })?;

            if !should_include_indexed_row(include_deleted, only_deleted, row.has_deleted_entries) {
                continue;
            }

            matched_rows.push(IndexedPathRow {
                path: row.path(),
                has_deleted_entries: row.has_deleted_entries,
            });
        }

        matched_rows
    };

    Ok(DriveQueryResult {
        loaded_rows,
        matched_rows,
    })
}

impl QueryArgs {
    fn should_use_columns(&self, stdout_is_terminal: bool) -> bool {
        match self.density {
            QueryDensity::Auto => stdout_is_terminal,
            QueryDensity::Lines => false,
            QueryDensity::Columns => true,
        }
    }

    fn invoke_indexed(self, mft_files: Vec<(char, PathBuf)>, sync_dir: &Path) -> eyre::Result<()> {
        let query_plan = {
            let _span = info_span!("parse_query_rules", query = ?self.query).entered();
            QueryPlan::parse_inputs(&self.query)?
        };
        let query_scope = {
            let _span = info_span!("resolve_query_scope", query_scope = ?self.r#in).entered();
            resolve_query_scope(self.r#in.as_deref())?
        };

        let mut loaded_rows = 0usize;
        let mut results = Vec::new();
        {
            let _span = info_span!("load_search_indexes", drives = mft_files.len()).entered();
            let include_deleted = self.include_deleted;
            let only_deleted = self.only_deleted;
            let load_results: Vec<eyre::Result<DriveQueryResult>> = mft_files
                .into_par_iter()
                .map(|(drive_letter, _)| {
                    load_and_query_drive_search_index(
                        drive_letter,
                        sync_dir,
                        &query_plan,
                        include_deleted,
                        only_deleted,
                    )
                })
                .collect();

            for result in load_results {
                let result = result?;
                loaded_rows += result.loaded_rows;
                results.extend(
                    result
                        .matched_rows
                        .into_iter()
                        .filter(|row| should_include_scope(&row.path, query_scope.as_ref())),
                );
            }
        }

        info!(
            loaded_rows = loaded_rows,
            matched = results.len(),
            total = loaded_rows,
            "Indexed query completed"
        );

        let stdout_is_terminal = std::io::stdout().is_terminal();
        let colorize = stdout_is_terminal && (self.include_deleted || self.only_deleted);
        let result_limit = if self.limit == 0 {
            results.len()
        } else {
            self.limit.min(results.len())
        };
        let display_results = &results[..result_limit];

        if self.should_use_columns(stdout_is_terminal) {
            print_results_columns(display_results, colorize);
        } else {
            print_results_lines(display_results, colorize);
        }

        Ok(())
    }

    /// Query indexed paths from `.mft_search_index` files.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is empty, sync directory cannot be retrieved,
    /// drive letters cannot be resolved, the query scope cannot be canonicalized,
    /// or if reading/parsing index files fails.
    #[instrument(level = "info", skip_all, fields(query = ?self.query, query_scope = ?self.r#in, limit = self.limit, include_deleted = self.include_deleted, only_deleted = self.only_deleted, density = ?self.density))]
    pub fn invoke(self) -> eyre::Result<()> {
        debug!("Running query with args: {:?}", self);
        if self.query.iter().all(|query| query.trim().is_empty()) {
            eyre::bail!("query string required")
        }
        let sync_dir = {
            let _span = info_span!("resolve_sync_dir").entered();
            try_get_sync_dir()?
        };

        let mft_files: Vec<(char, PathBuf)> = {
            let _span = info_span!("discover_mft_files").entered();
            self.drive_letter_pattern
                .into_drive_letters()?
                .into_iter()
                .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
                .filter(|(_, p)| p.is_file())
                .collect()
        };
        self.invoke_indexed(mft_files, &sync_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::matching_row_indices_for_rule;
    use super::resolve_query_scope;
    use super::should_include_scope;
    use crate::query::QueryPlan;
    use crate::query::QueryRule;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
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

    fn parse_fixture_index()
    -> eyre::Result<crate::search_index::search_index_bytes::ParsedSearchIndex<'static>> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\src\\flower.jar"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\flowchart.txt"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\trees.zip"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        SearchIndexBytes::new(bytes).parse_trusted_for_query()
    }

    #[test]
    fn contains_rules_return_rows_from_trigram_candidates() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = QueryRule::parse("ower").expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0]);

        Ok(())
    }

    #[test]
    fn short_contains_rules_still_match_without_trigrams() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = QueryRule::parse("fl").expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0, 1]);

        Ok(())
    }

    #[test]
    fn query_plan_intersects_contains_and_suffix_candidates() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let plan = QueryPlan::parse_inputs(&[String::from("flow .jar$")])?;

        assert_eq!(
            plan.matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed, rule))?,
            vec![0]
        );

        Ok(())
    }

    #[test]
    fn suffix_rules_match_only_terminal_segments_in_indexed_queries() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\repo\\project.git"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\.git\\objects\\pack\\pack-a.rev"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\.git\\refs\\remotes\\origin\\main"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        let parsed = SearchIndexBytes::new(bytes).parse_trusted_for_query()?;
        let rule = QueryRule::parse(".git$").expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0]);

        Ok(())
    }

    #[test]
    fn query_scope_directory_matches_descendants_but_not_sibling_prefixes() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_dir = temp_dir.path().join("repo");
        let nested_file = scope_dir.join("music").join("song.mp3");
        let sibling_file = temp_dir.path().join("repo2").join("song.mp3");

        std::fs::create_dir_all(nested_file.parent().expect("nested file should have parent"))?;
        std::fs::create_dir_all(sibling_file.parent().expect("sibling file should have parent"))?;
        std::fs::write(&nested_file, [])?;
        std::fs::write(&sibling_file, [])?;

        let scope = resolve_query_scope(Some(&scope_dir.to_string_lossy()))?
            .expect("directory scope should resolve");

        assert!(should_include_scope(&nested_file.to_string_lossy(), Some(&scope)));
        assert!(!should_include_scope(&sibling_file.to_string_lossy(), Some(&scope)));

        Ok(())
    }

    #[test]
    fn query_scope_file_matches_only_exact_path() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_file = temp_dir.path().join("track.flac");
        let other_file = temp_dir.path().join("track.flac.bak");

        std::fs::write(&scope_file, [])?;
        std::fs::write(&other_file, [])?;

        let scope = resolve_query_scope(Some(&scope_file.to_string_lossy()))?
            .expect("file scope should resolve");

        assert!(should_include_scope(&scope_file.to_string_lossy(), Some(&scope)));
        assert!(!should_include_scope(&other_file.to_string_lossy(), Some(&scope)));

        Ok(())
    }

    #[test]
    fn query_scope_dot_resolves_against_current_working_directory() -> eyre::Result<()> {
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
}
