use crate::query::QueryRule;
use crate::search_index::search_index_bytes::ParsedSearchIndex;
use eyre::Context;
use tracing::info_span;

pub fn matching_row_indices_for_rule(
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

#[cfg(test)]
mod tests {
    use super::matching_row_indices_for_rule;
    use crate::query::QueryRule;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::ParsedSearchIndex;
    use crate::search_index::search_index_bytes::SearchIndexBytes;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;

    fn parse_index(rows: &[SearchIndexPathRow]) -> eyre::Result<ParsedSearchIndex<'static>> {
        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        SearchIndexBytes::new(bytes).parse_trusted_for_query()
    }

    fn parse_fixture_index() -> eyre::Result<ParsedSearchIndex<'static>> {
        parse_index(&[
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
        ])
    }

    #[test]
    fn contains_rules_return_rows_from_trigram_candidates() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = "ower".parse::<QueryRule>().expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0]);

        Ok(())
    }

    #[test]
    fn prefix_rules_match_segment_prefixes_in_indexed_queries() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = "<flo".parse::<QueryRule>().expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0, 1]);

        Ok(())
    }

    #[test]
    fn short_contains_rules_still_match_without_trigrams() -> eyre::Result<()> {
        let parsed = parse_fixture_index()?;
        let rule = "fl".parse::<QueryRule>().expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0, 1]);

        Ok(())
    }

    #[test]
    fn suffix_rules_match_only_terminal_segments_in_indexed_queries() -> eyre::Result<()> {
        let parsed = parse_index(&[
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
        ])?;
        let rule = ".git>".parse::<QueryRule>().expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0]);

        Ok(())
    }

    #[test]
    fn exact_rules_match_only_terminal_segments_in_indexed_queries() -> eyre::Result<()> {
        let parsed = parse_index(&[
            SearchIndexPathRow {
                path: String::from("C:\\repo\\package.json"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\my-package.json"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\package.json.backup"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\package.json\\README.md"),
                has_deleted_entries: false,
            },
        ])?;
        let rule = "<package.json>"
            .parse::<QueryRule>()
            .expect("rule should parse");

        assert_eq!(matching_row_indices_for_rule(&parsed, &rule)?, vec![0]);

        Ok(())
    }
}
