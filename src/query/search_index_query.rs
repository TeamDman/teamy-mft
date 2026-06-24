use crate::machine::config::published_drive_paths;
use crate::query::MatchingRowIndices;
use crate::query::Pathlike;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::query::matching_row_indices_for_rule;
use crate::query::query_scope::QueryScope;
use crate::search_index::load::MappedSearchIndex;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use eyre::Context;
use std::ops::ControlFlow;
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

fn matching_row_indices_for_scope_component(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    normalized_component: &str,
) -> eyre::Result<MatchingRowIndices> {
    let mut row_indices = Vec::new();

    for (segment_id, segment) in parsed_index.segments().iter().enumerate() {
        if segment.normalized != normalized_component {
            continue;
        }

        let segment_id = u32::try_from(segment_id).wrap_err_with(|| {
            format!("Segment id {segment_id} does not fit into u32 for scope candidate lookup")
        })?;
        row_indices.extend(parsed_index.postings(segment_id)?);
    }

    row_indices.sort_unstable();
    row_indices.dedup();

    Ok(MatchingRowIndices::RowIndices(row_indices))
}

fn matching_row_indices_for_scope(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    scope: &QueryScope,
) -> eyre::Result<MatchingRowIndices> {
    let scope_components = scope.normalized_components();
    let mut coarse_scope_components = scope_components.clone();
    coarse_scope_components.sort_unstable();
    coarse_scope_components.dedup();

    if coarse_scope_components.is_empty() {
        let row_count = u32::try_from(parsed_index.row_count())
            .wrap_err("Parsed search index row count does not fit into u32")?;
        return Ok(MatchingRowIndices::MatchAll { row_count });
    }

    let coarse_matches = {
        let _span = info_span!("match_query_scope_components").entered();
        let mut matches: Option<MatchingRowIndices> = None;

        for component in &coarse_scope_components {
            let component_matches =
                matching_row_indices_for_scope_component(parsed_index, component.as_str())?;
            matches = Some(match matches.take() {
                Some(existing) => existing.intersect(component_matches),
                None => component_matches,
            });

            if matches.as_ref().is_some_and(MatchingRowIndices::is_empty) {
                break;
            }
        }

        matches.unwrap_or(MatchingRowIndices::RowIndices(Vec::new()))
    };

    let MatchingRowIndices::RowIndices(row_indices) = coarse_matches else {
        return Ok(coarse_matches);
    };

    info_span!("filter_query_scope_candidates").in_scope(|| {
        let mut exact_matches = Vec::with_capacity(row_indices.len());

        for row_index in row_indices {
            if parsed_index.row_matches_normalized_path_components(
                row_index,
                &scope_components,
                scope.include_descendants,
            )? {
                exact_matches.push(row_index);
            }
        }

        Ok(MatchingRowIndices::RowIndices(exact_matches))
    })
}

fn matching_row_indices_for_scopes(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    scopes: &[QueryScope],
) -> eyre::Result<MatchingRowIndices> {
    let mut matches: Option<MatchingRowIndices> = None;

    for scope in scopes {
        let scope_matches = matching_row_indices_for_scope(parsed_index, scope)?;
        matches = Some(match matches.take() {
            Some(existing) => existing.union(scope_matches),
            None => scope_matches,
        });
    }

    Ok(matches.unwrap_or(MatchingRowIndices::RowIndices(Vec::new())))
}

fn visit_matching_row_indices(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    query_plan: &QueryPlan,
    scopes: &[QueryScope],
    mut visit: impl FnMut(u32) -> eyre::Result<ControlFlow<(), ()>>,
) -> eyre::Result<ControlFlow<(), ()>> {
    let scope_matches = (!scopes.is_empty())
        .then(|| matching_row_indices_for_scopes(parsed_index, scopes))
        .transpose()?;

    let matched_rows = {
        let _span = info_span!("match_search_index_postings").entered();
        let query_matches = query_plan.query.matching_row_index_candidates(&|rule| {
            #[cfg(feature = "extended_observability")]
            let _span = tracing::debug_span!("match_search_index_rule").entered();
            matching_row_indices_for_rule(parsed_index, rule)
        })?;

        match scope_matches {
            Some(scope_matches) => query_matches.intersect(scope_matches),
            None => query_matches,
        }
    };

    match matched_rows {
        MatchingRowIndices::MatchAll { row_count } => {
            let _span = info_span!("visit_all_matched_row_indices").entered();
            for row_index in 0..row_count {
                let control_flow = {
                    #[cfg(feature = "extended_observability_per_record")]
                    let _span = tracing::debug_span!("visit_all_matched_row_index").entered();
                    visit(row_index)?
                };
                if control_flow == ControlFlow::Break(()) {
                    return Ok(ControlFlow::Break(()));
                }
            }

            Ok(ControlFlow::Continue(()))
        }
        MatchingRowIndices::RowIndices(row_indices) => {
            let _span = info_span!("visit_matched_row_indices").entered();
            for row_index in row_indices {
                if visit(row_index)? == ControlFlow::Break(()) {
                    return Ok(ControlFlow::Break(()));
                }
            }
            Ok(ControlFlow::Continue(()))
        }
    }
}

pub(crate) fn visit_parsed_search_index_rows(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    query_plan: &QueryPlan,
    scopes: &[QueryScope],
    include_deleted: bool,
    only_deleted: bool,
    mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
) -> eyre::Result<(usize, ControlFlow<(), ()>)> {
    let loaded_rows = parsed_index.row_count();
    let _span = info_span!("materialize_matched_index_rows").entered();
    let control_flow = {
        let _span = info_span!("visit_matched_index_rows").entered();
        visit_matching_row_indices(parsed_index, query_plan, scopes, |row_index| {
            let (path, has_deleted_entries) = {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("read_matched_index_row").entered();
                let row = parsed_index.row_view(row_index as usize)?;
                (row.path(), row.has_deleted_entries)
            };

            let should_include = {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("evaluate_deleted_row_filter").entered();
                should_include_indexed_row(include_deleted, only_deleted, has_deleted_entries)
            };

            if !should_include {
                return Ok(ControlFlow::Continue(()));
            }
            {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("emit_matched_index_row").entered();
                visit(QueryResultRow {
                    path,
                    has_deleted_entries,
                    is_filtered: false,
                })
            }
        })
    }?;

    if control_flow == ControlFlow::Break(()) {
        return Ok((loaded_rows, ControlFlow::Break(())));
    }

    Ok((loaded_rows, ControlFlow::Continue(())))
}

pub(crate) fn visit_matching_parsed_row_indices(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    query_plan: &QueryPlan,
    scopes: &[QueryScope],
    include_deleted: bool,
    only_deleted: bool,
    mut visit: impl FnMut(u32) -> eyre::Result<ControlFlow<(), ()>>,
) -> eyre::Result<(usize, ControlFlow<(), ()>)> {
    let loaded_rows = parsed_index.row_count();
    let control_flow = {
        let _span = info_span!("visit_matching_row_indices_only").entered();
        visit_matching_row_indices(parsed_index, query_plan, scopes, |row_index| {
            let has_deleted_entries = {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("read_matched_row_index_filter_state").entered();
                parsed_index
                    .row_view(row_index as usize)?
                    .has_deleted_entries
            };

            let should_include = {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("evaluate_matched_row_index_filter").entered();
                should_include_indexed_row(include_deleted, only_deleted, has_deleted_entries)
            };

            if !should_include {
                return Ok(ControlFlow::Continue(()));
            }

            {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("emit_matched_row_index").entered();
                visit(row_index)
            }
        })
    }?;

    if control_flow == ControlFlow::Break(()) {
        return Ok((loaded_rows, ControlFlow::Break(())));
    }

    Ok((loaded_rows, ControlFlow::Continue(())))
}

fn visit_matching_search_index_rows(
    index_path: &Path,
    query_plan: &QueryPlan,
    scopes: &[QueryScope],
    include_deleted: bool,
    only_deleted: bool,
    visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
) -> eyre::Result<(usize, ControlFlow<(), ()>)> {
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

    visit_parsed_search_index_rows(
        &parsed_index,
        query_plan,
        scopes,
        include_deleted,
        only_deleted,
        visit,
    )
    .wrap_err_with(|| {
        format!(
            "Failed visiting matched search index rows from {}",
            index_path.display()
        )
    })
}

pub(crate) fn mapped_search_index_has_rows(mapped: &MappedSearchIndex) -> bool {
    mapped.header.node_count > 0
}

fn search_index_has_rows(index_path: &Path) -> eyre::Result<bool> {
    let mapped = MappedSearchIndex::open(index_path)
        .wrap_err_with(|| format!("Failed loading search index from {}", index_path.display()))?;
    Ok(mapped_search_index_has_rows(&mapped))
}

fn collect_matching_row_refs(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    query_plan: &QueryPlan,
    scopes: &[QueryScope],
    include_deleted: bool,
    only_deleted: bool,
) -> eyre::Result<(usize, Vec<MatchingRowRef>)> {
    let _span = info_span!("collect_matching_row_refs").entered();
    let mut rows = Vec::new();
    let (loaded_rows, control_flow) = visit_matching_parsed_row_indices(
        parsed_index,
        query_plan,
        scopes,
        include_deleted,
        only_deleted,
        |row_index| {
            let path = {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("read_matching_row_ref_path").entered();
                parsed_index.row_view(row_index as usize)?.path()
            };

            {
                #[cfg(feature = "extended_observability_per_record")]
                let _span = tracing::debug_span!("push_matching_row_ref").entered();
                rows.push(MatchingRowRef { row_index, path });
            };
            Ok(ControlFlow::Continue(()))
        },
    )?;

    if control_flow == ControlFlow::Break(()) {
        return Ok((loaded_rows, rows));
    }

    Ok((loaded_rows, rows))
}

fn materialize_row(
    parsed_index: &crate::search_index::search_index_bytes::ParsedSearchIndex<'_>,
    row_index: u32,
) -> eyre::Result<QueryResultRow> {
    let row = parsed_index.row_view(row_index as usize)?;
    Ok(QueryResultRow {
        path: row.path(),
        has_deleted_entries: row.has_deleted_entries,
        is_filtered: false,
    })
}

pub(crate) fn visit_drive_search_index_rows(
    drive_letter: char,
    sync_dir: &Path,
    query_plan: &QueryPlan,
    scopes: &[QueryScope],
    include_deleted: bool,
    only_deleted: bool,
    mut visit: impl FnMut(QueryResultRow) -> eyre::Result<ControlFlow<(), ()>>,
) -> eyre::Result<usize> {
    let paths = published_drive_paths(sync_dir, drive_letter);

    if paths.overlay_index_path.is_file() && search_index_has_rows(&paths.overlay_index_path)? {
        let base_mapped = MappedSearchIndex::open(&paths.base_index_path).wrap_err_with(|| {
            format!(
                "Fast query requires {}. Run `teamy-mft sync index --drive-pattern {}` first.",
                paths.base_index_path.display(),
                drive_letter,
            )
        })?;
        let base_parsed = SearchIndexBytes::new(base_mapped.bytes())
            .parse_trusted_for_query()
            .wrap_err_with(|| {
                format!(
                    "Failed preparing search index rows from {}",
                    paths.base_index_path.display()
                )
            })?;
        let overlay_mapped =
            MappedSearchIndex::open(&paths.overlay_index_path).wrap_err_with(|| {
                format!(
                    "Failed loading search index from {}",
                    paths.overlay_index_path.display()
                )
            })?;
        let overlay_parsed = SearchIndexBytes::new(overlay_mapped.bytes())
            .parse_trusted_for_query()
            .wrap_err_with(|| {
                format!(
                    "Failed preparing search index rows from {}",
                    paths.overlay_index_path.display()
                )
            })?;

        let (base_loaded_rows, mut base_rows) = collect_matching_row_refs(
            &base_parsed,
            query_plan,
            scopes,
            include_deleted,
            only_deleted,
        )?;
        let (overlay_loaded_rows, mut overlay_rows) = collect_matching_row_refs(
            &overlay_parsed,
            query_plan,
            scopes,
            include_deleted,
            only_deleted,
        )?;

        base_rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        overlay_rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));

        let mut base_offset = 0_usize;
        let mut overlay_offset = 0_usize;
        while base_offset < base_rows.len() || overlay_offset < overlay_rows.len() {
            let row = match (base_rows.get(base_offset), overlay_rows.get(overlay_offset)) {
                (Some(base_row), Some(overlay_row)) => {
                    if overlay_row.path <= base_row.path {
                        if overlay_row.path == base_row.path {
                            base_offset += 1;
                        }
                        overlay_offset += 1;
                        materialize_row(&overlay_parsed, overlay_row.row_index)?
                    } else {
                        base_offset += 1;
                        materialize_row(&base_parsed, base_row.row_index)?
                    }
                }
                (Some(base_row), None) => {
                    base_offset += 1;
                    materialize_row(&base_parsed, base_row.row_index)?
                }
                (None, Some(overlay_row)) => {
                    overlay_offset += 1;
                    materialize_row(&overlay_parsed, overlay_row.row_index)?
                }
                (None, None) => break,
            };
            if visit(row)? == ControlFlow::Break(()) {
                break;
            }
        }
        return Ok(base_loaded_rows + overlay_loaded_rows);
    }

    let (loaded_rows, _) = visit_matching_search_index_rows(
        &paths.base_index_path,
        query_plan,
        scopes,
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

struct MatchingRowRef {
    row_index: u32,
    path: Pathlike,
}

#[cfg(test)]
mod tests {
    use super::visit_matching_parsed_row_indices;
    use crate::query::QueryPlan;
    use crate::query::resolve_query_scopes;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::ParsedSearchIndex;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;
    use std::ops::ControlFlow;

    fn parse_index(rows: &[SearchIndexPathRow]) -> eyre::Result<ParsedSearchIndex<'static>> {
        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        SearchIndexBytes::new(bytes).parse_trusted_for_query()
    }

    #[cfg(windows)]
    #[test]
    fn scope_prefilter_excludes_out_of_scope_match_all_rows() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let scope_dir = temp_dir.path().join("alpha").join("beta").join("gamma");
        let in_scope_file = scope_dir.join("match-one.txt");
        let nested_in_scope_file = scope_dir.join("nested").join("match-two.txt");
        let out_of_scope_same_components = temp_dir
            .path()
            .join("alpha")
            .join("other")
            .join("gamma")
            .join("beta")
            .join("mismatch.txt");

        std::fs::create_dir_all(
            in_scope_file
                .parent()
                .expect("in-scope file should have parent"),
        )?;
        std::fs::create_dir_all(
            nested_in_scope_file
                .parent()
                .expect("nested in-scope file should have parent"),
        )?;
        std::fs::create_dir_all(
            out_of_scope_same_components
                .parent()
                .expect("out-of-scope file should have parent"),
        )?;
        std::fs::write(&in_scope_file, [])?;
        std::fs::write(&nested_in_scope_file, [])?;
        std::fs::write(&out_of_scope_same_components, [])?;

        let in_scope_file = dunce::canonicalize(in_scope_file)?;
        let nested_in_scope_file = dunce::canonicalize(nested_in_scope_file)?;
        let out_of_scope_same_components = dunce::canonicalize(out_of_scope_same_components)?;

        let parsed = parse_index(&[
            SearchIndexPathRow {
                path: in_scope_file.to_string_lossy().to_string().into(),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: out_of_scope_same_components
                    .to_string_lossy()
                    .to_string()
                    .into(),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: nested_in_scope_file.to_string_lossy().to_string().into(),
                has_deleted_entries: false,
            },
        ])?;

        let mut plan = QueryPlan::parse_inputs(&[String::from("<>")])?;
        plan.r#in = vec![scope_dir.to_string_lossy().to_string()];
        let scopes = resolve_query_scopes(&plan.r#in)?;

        let mut matching_rows = Vec::new();
        let (_, control_flow) = visit_matching_parsed_row_indices(
            &parsed,
            &plan,
            scopes.as_slice(),
            false,
            false,
            |row_index| {
                matching_rows.push(row_index);
                Ok(ControlFlow::Continue(()))
            },
        )?;

        assert_eq!(control_flow, ControlFlow::Continue(()));
        assert_eq!(matching_rows, vec![0, 2]);

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn multiple_scopes_union_prefilter_matches() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let alpha_dir = temp_dir.path().join("alpha");
        let beta_dir = temp_dir.path().join("beta");
        let alpha_file = alpha_dir.join("match-one.txt");
        let beta_file = beta_dir.join("match-two.txt");
        let out_of_scope_file = temp_dir.path().join("gamma").join("miss.txt");

        std::fs::create_dir_all(alpha_file.parent().expect("alpha file should have parent"))?;
        std::fs::create_dir_all(beta_file.parent().expect("beta file should have parent"))?;
        std::fs::create_dir_all(
            out_of_scope_file
                .parent()
                .expect("out-of-scope file should have parent"),
        )?;
        std::fs::write(&alpha_file, [])?;
        std::fs::write(&beta_file, [])?;
        std::fs::write(&out_of_scope_file, [])?;

        let alpha_file = dunce::canonicalize(alpha_file)?;
        let beta_file = dunce::canonicalize(beta_file)?;
        let out_of_scope_file = dunce::canonicalize(out_of_scope_file)?;

        let parsed = parse_index(&[
            SearchIndexPathRow {
                path: alpha_file.to_string_lossy().to_string().into(),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: out_of_scope_file.to_string_lossy().to_string().into(),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: beta_file.to_string_lossy().to_string().into(),
                has_deleted_entries: false,
            },
        ])?;

        let mut plan = QueryPlan::parse_inputs(&[String::from("<>")])?;
        plan.r#in = vec![
            alpha_dir.to_string_lossy().to_string(),
            beta_dir.to_string_lossy().to_string(),
        ];
        let scopes = resolve_query_scopes(&plan.r#in)?;

        let mut matching_rows = Vec::new();
        let (_, control_flow) = visit_matching_parsed_row_indices(
            &parsed,
            &plan,
            scopes.as_slice(),
            false,
            false,
            |row_index| {
                matching_rows.push(row_index);
                Ok(ControlFlow::Continue(()))
            },
        )?;

        assert_eq!(control_flow, ControlFlow::Continue(()));
        assert_eq!(matching_rows, vec![0, 2]);

        Ok(())
    }
}
