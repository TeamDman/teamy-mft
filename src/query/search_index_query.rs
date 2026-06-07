use crate::machine::config::published_drive_paths;
use crate::query::ControlFlow;
use crate::query::DriveQueryResult;
use crate::query::Pathlike;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::matching_row_indices_for_rule;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use eyre::Context;
use std::collections::BTreeMap;
use std::path::Path;
use tracing::info_span;

fn should_include_indexed_row(
    include_deleted: bool,
    only_deleted: bool,
    has_deleted_entries: bool,
) -> bool {
    // cli[impl command.query.deleted-filter]
    if only_deleted {
        return has_deleted_entries;
    }

    include_deleted || !has_deleted_entries
}

fn load_and_query_search_index(
    index_path: &Path,
    _drive_letter: char,
    _index_kind: &'static str,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<DriveQueryResult> {
    let _span = info_span!("load_drive_search_index").entered();
    {
        let _span = info_span!("validate_search_index_file").entered();
        if !index_path.is_file() {
            eyre::bail!("Fast query requires {}.", index_path.display(),);
        }
    }

    let mapped = {
        let _span = info_span!("map_search_index_file").entered();
        MappedSearchIndex::open(index_path).wrap_err_with(|| {
            format!("Failed loading search index from {}", index_path.display())
        })?
    };

    let parsed_index = {
        let _span = info_span!("parse_search_index_for_query").entered();
        SearchIndexBytes::new(mapped.bytes())
            .parse_trusted_for_query()
            .wrap_err_with(|| {
                format!(
                    "Failed preparing search index rows from {}",
                    index_path.display()
                )
            })?
    };

    let loaded_rows = parsed_index.row_count();
    let matched_row_indices = {
        let _span = info_span!("match_search_index_postings").entered();
        query_plan
            .query
            .matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed_index, rule))
            .wrap_err_with(|| {
                format!(
                    "Failed matching search index rows from {}",
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
                        "Failed materializing search index row {} from {}",
                        row_index,
                        index_path.display()
                    )
                })?;

            if !should_include_indexed_row(include_deleted, only_deleted, row.has_deleted_entries) {
                continue;
            }

            matched_rows.push(QueryResultRow {
                path: Pathlike::from(row.path()),
                has_deleted_entries: row.has_deleted_entries,
                is_filtered: false,
            });
        }

        matched_rows
    };

    Ok(DriveQueryResult {
        loaded_rows,
        matched_rows,
    })
}

fn visit_matching_search_index_rows(
    index_path: &Path,
    _drive_letter: char,
    _index_kind: &'static str,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
    mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow>,
) -> eyre::Result<(usize, ControlFlow)> {
    let _span = info_span!("load_drive_search_index").entered();
    {
        let _span = info_span!("validate_search_index_file").entered();
        if !index_path.is_file() {
            eyre::bail!("Fast query requires {}.", index_path.display(),);
        }
    }

    let mapped = {
        let _span = info_span!("map_search_index_file").entered();
        MappedSearchIndex::open(index_path).wrap_err_with(|| {
            format!("Failed loading search index from {}", index_path.display())
        })?
    };

    let parsed_index = {
        let _span = info_span!("parse_search_index_for_query").entered();
        SearchIndexBytes::new(mapped.bytes())
            .parse_trusted_for_query()
            .wrap_err_with(|| {
                format!(
                    "Failed preparing search index rows from {}",
                    index_path.display()
                )
            })?
    };

    let loaded_rows = parsed_index.row_count();
    let matched_row_indices = {
        let _span = info_span!("match_search_index_postings").entered();
        query_plan
            .query
            .matching_row_indices(&|rule| matching_row_indices_for_rule(&parsed_index, rule))
            .wrap_err_with(|| {
                format!(
                    "Failed matching search index rows from {}",
                    index_path.display()
                )
            })?
    };

    let _span = info_span!("materialize_matched_index_rows").entered();
    for row_index in matched_row_indices {
        let row = parsed_index
            .row_view(row_index as usize)
            .wrap_err_with(|| {
                format!(
                    "Failed materializing search index row {} from {}",
                    row_index,
                    index_path.display()
                )
            })?;

        if !should_include_indexed_row(include_deleted, only_deleted, row.has_deleted_entries) {
            continue;
        }

        let control_flow = visit(QueryResultRow {
            path: Pathlike::from(row.path()),
            has_deleted_entries: row.has_deleted_entries,
            is_filtered: false,
        })?;

        if control_flow == ControlFlow::Break {
            return Ok((loaded_rows, ControlFlow::Break));
        }
    }

    Ok((loaded_rows, ControlFlow::Continue))
}

fn search_index_has_rows(index_path: &Path) -> eyre::Result<bool> {
    let mapped = MappedSearchIndex::open(index_path)
        .wrap_err_with(|| format!("Failed loading search index from {}", index_path.display()))?;
    Ok(SearchIndexBytes::new(mapped.bytes()).header()?.node_count > 0)
}

pub(crate) fn load_and_query_drive_search_index(
    drive_letter: char,
    sync_dir: &Path,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<DriveQueryResult> {
    let paths = published_drive_paths(sync_dir, drive_letter);
    let mut result = load_and_query_search_index(
        &paths.base_index_path,
        drive_letter,
        "base",
        query_plan,
        include_deleted,
        only_deleted,
    )
    .wrap_err_with(|| {
        format!(
            "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
            paths.base_index_path.display(),
            drive_letter
        )
    })?;

    if paths.overlay_index_path.is_file() {
        let overlay_result = load_and_query_search_index(
            &paths.overlay_index_path,
            drive_letter,
            "overlay",
            query_plan,
            include_deleted,
            only_deleted,
        )?;
        result.loaded_rows += overlay_result.loaded_rows;
        result.matched_rows = merge_rows(result.matched_rows, overlay_result.matched_rows);
    }

    Ok(result)
}

pub(crate) fn visit_drive_search_index_rows(
    drive_letter: char,
    sync_dir: &Path,
    query_plan: &QueryPlan,
    include_deleted: bool,
    only_deleted: bool,
    mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow>,
) -> eyre::Result<usize> {
    let paths = published_drive_paths(sync_dir, drive_letter);

    if paths.overlay_index_path.is_file() && search_index_has_rows(&paths.overlay_index_path)? {
        let result = load_and_query_drive_search_index(
            drive_letter,
            sync_dir,
            query_plan,
            include_deleted,
            only_deleted,
        )?;
        let loaded_rows = result.loaded_rows;
        for row in result.matched_rows {
            if visit(row)? == ControlFlow::Break {
                break;
            }
        }
        return Ok(loaded_rows);
    }

    let (loaded_rows, _) = visit_matching_search_index_rows(
        &paths.base_index_path,
        drive_letter,
        "base",
        query_plan,
        include_deleted,
        only_deleted,
        &mut visit,
    )
    .wrap_err_with(|| {
        format!(
            "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
            paths.base_index_path.display(),
            drive_letter
        )
    })?;

    Ok(loaded_rows)
}

pub(crate) fn merge_rows(
    base_rows: Vec<QueryResultRow>,
    overlay_rows: Vec<QueryResultRow>,
) -> Vec<QueryResultRow> {
    let mut merged = BTreeMap::<Pathlike, QueryResultRow>::new();
    for row in base_rows {
        merged.insert(row.path.clone(), row);
    }
    for row in overlay_rows {
        merged.insert(row.path.clone(), row);
    }
    merged.into_values().collect()
}
