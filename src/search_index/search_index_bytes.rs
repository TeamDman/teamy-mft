use crate::search_index::format::SEARCH_INDEX_HEADER_LEN;
use crate::search_index::format::SEARCH_INDEX_MAGIC;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use eyre::Context;
use eyre::bail;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tracing::info_span;
use zerotrie::ZeroTriePerfectHash;

const SEARCH_INDEX_BODY_PREFIX_LEN: usize = 4 + 4 + 4 + 4 + 4 + 4 + 4 + 4;
const SEARCH_INDEX_SEGMENT_LEN_PREFIX: usize = 4 + 4;
const SEARCH_INDEX_EXTENSION_SUFFIX_LEN_PREFIX: usize = 4;
const SEARCH_INDEX_TRIGRAM_LEN: usize = 3;
const SEARCH_INDEX_NODE_LEN: usize = 4 + 4;
const SEARCH_INDEX_TERMINAL_LEN: usize = 4 + 1;
const SEARCH_INDEX_POSTING_RANGE_LEN: usize = 4 + 4;
const SEARCH_INDEX_POSTING_ROW_LEN: usize = 4;
const NO_PARENT_NODE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexBytes<'a> {
    bytes: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct SearchIndexBytesMut {
    header: SearchIndexHeader,
    rows: Vec<SearchIndexPathRow>,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexPathSegmentView<'a> {
    display_bytes: &'a [u8],
    pub normalized: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct SearchIndexExtensionSuffixView<'a> {
    pub normalized_suffix: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct SearchIndexBodyPrefix<'a> {
    cursor: usize,
    segment_count: usize,
    extension_count: usize,
    trigram_count: usize,
    path_node_count: usize,
    posting_row_id_count: usize,
    extension_posting_row_id_count: usize,
    trigram_posting_segment_id_count: usize,
    trie_bytes: &'a [u8],
}

impl<'a> SearchIndexPathSegmentView<'a> {
    #[must_use]
    pub fn display_bytes(&self) -> &'a [u8] {
        self.display_bytes
    }

    #[must_use]
    pub fn display_lossy(&self) -> Cow<'a, str> {
        String::from_utf8_lossy(self.display_bytes)
    }
}

#[derive(Debug, Clone)]
pub struct ParsedSearchIndex<'a> {
    bytes: &'a [u8],
    trie_bytes: &'a [u8],
    segments: Arc<[SearchIndexPathSegmentView<'a>]>,
    extension_suffixes: Arc<[SearchIndexExtensionSuffixView<'a>]>,
    trigrams: Arc<[[u8; SEARCH_INDEX_TRIGRAM_LEN]]>,
    path_node_offset: usize,
    terminal_offset: usize,
    terminal_count: usize,
    posting_range_offset: usize,
    posting_row_id_offset: usize,
    posting_row_id_count: usize,
    extension_posting_range_offset: usize,
    extension_posting_row_id_offset: usize,
    extension_posting_row_id_count: usize,
    trigram_posting_range_offset: usize,
    trigram_posting_segment_id_offset: usize,
    trigram_posting_segment_id_count: usize,
}

#[derive(Debug, Clone)]
pub struct SearchIndexPathRowView<'a> {
    parsed: ParsedSearchIndex<'a>,
    terminal_node_index: u32,
    pub has_deleted_entries: bool,
}

impl SearchIndexPathRowView<'_> {
    #[must_use]
    pub fn path(&self) -> String {
        let mut segments = self.segment_views().collect::<Vec<_>>();
        segments.reverse();

        let path_len = segments
            .iter()
            .map(|segment| segment.display_bytes.len())
            .sum::<usize>()
            + segments.len().saturating_sub(1);
        let mut path = String::with_capacity(path_len);
        for (index, segment) in segments.into_iter().enumerate() {
            if index > 0 {
                path.push('\\');
            }
            path.push_str(&segment.display_lossy());
        }
        path
    }

    #[must_use]
    pub fn segment_views(&self) -> SearchIndexPathSegmentIter<'_> {
        SearchIndexPathSegmentIter {
            parsed: self.parsed.clone(),
            next_node_index: Some(self.terminal_node_index),
        }
    }

    #[must_use]
    pub fn to_owned(self) -> SearchIndexPathRow {
        SearchIndexPathRow {
            path: self.path(),
            has_deleted_entries: self.has_deleted_entries,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchIndexPathSegmentIter<'a> {
    parsed: ParsedSearchIndex<'a>,
    next_node_index: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SearchIndexRowIter<'a> {
    parsed: ParsedSearchIndex<'a>,
    next_terminal_offset: usize,
    remaining_rows: usize,
    row_index: usize,
}

#[derive(Debug, Clone)]
pub struct SearchIndexPostingIter<'a> {
    bytes: &'a [u8],
    next_offset: usize,
    remaining_rows: usize,
}

impl<'a> SearchIndexBytes<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    #[must_use]
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// # Errors
    ///
    /// Returns an error if the index body is truncated, malformed, or contains
    /// invalid UTF-8 payloads.
    pub fn parse(&self) -> eyre::Result<ParsedSearchIndex<'a>> {
        self.parse_body(ParseMode::Validated)
    }

    /// # Errors
    ///
    /// Returns an error if the index header/body prefix or segment payloads are
    /// truncated or malformed. This skips the expensive full-table structural
    /// validation passes and is intended for trusted query-time reads of
    /// search indexes written by this binary.
    pub fn parse_trusted_for_query(&self) -> eyre::Result<ParsedSearchIndex<'a>> {
        self.parse_body(ParseMode::Trusted)
    }

    /// # Errors
    ///
    /// Returns an error if the bytes are too short to contain a full header or
    /// if the header magic does not match the expected search index magic.
    pub fn header(&self) -> eyre::Result<SearchIndexHeader> {
        if self.bytes.len() < SEARCH_INDEX_HEADER_LEN {
            bail!(
                "Invalid search index header: expected at least {} bytes, got {}",
                SEARCH_INDEX_HEADER_LEN,
                self.bytes.len()
            );
        }

        if &self.bytes[..SEARCH_INDEX_MAGIC.len()] != SEARCH_INDEX_MAGIC {
            bail!("Invalid search index header magic");
        }

        let mut cursor = SEARCH_INDEX_MAGIC.len();
        let version = read_u16(self.bytes, &mut cursor);
        let flags = read_u16(self.bytes, &mut cursor);
        let drive_letter = self.bytes[cursor];
        cursor += 1;
        let source_mft_len_bytes = read_u64(self.bytes, &mut cursor);
        let node_count = read_u64(self.bytes, &mut cursor);

        Ok(SearchIndexHeader {
            version,
            flags,
            drive_letter,
            source_mft_len_bytes,
            node_count,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the header cannot be parsed.
    pub fn version(&self) -> eyre::Result<u16> {
        Ok(self.header()?.version)
    }

    /// # Errors
    ///
    /// Returns an error if the index body is truncated, malformed, or contains
    /// invalid UTF-8 payloads.
    pub fn row_views(&self) -> eyre::Result<SearchIndexRowIter<'a>> {
        Ok(self.parse_body(ParseMode::Validated)?.row_views())
    }

    /// # Errors
    ///
    /// Returns an error if the index body cannot be parsed or `row_index` is
    /// out of bounds.
    pub fn row_view(&self, row_index: usize) -> eyre::Result<SearchIndexPathRowView<'a>> {
        self.parse_body(ParseMode::Validated)?.row_view(row_index)
    }

    /// # Errors
    ///
    /// Returns an error if the index body cannot be parsed.
    pub fn trie(&self) -> eyre::Result<&'a ZeroTriePerfectHash<[u8]>> {
        Ok(self.parse_body(ParseMode::Validated)?.trie())
    }

    fn parse_body(&self, mode: ParseMode) -> eyre::Result<ParsedSearchIndex<'a>> {
        if self.bytes.len() < SEARCH_INDEX_HEADER_LEN + SEARCH_INDEX_BODY_PREFIX_LEN {
            bail!(
                "Invalid search index body: expected at least {} bytes after header, got {}",
                SEARCH_INDEX_BODY_PREFIX_LEN,
                self.bytes.len().saturating_sub(SEARCH_INDEX_HEADER_LEN)
            );
        }

        let terminal_count = {
            let _span = info_span!("parse_search_index_header").entered();
            terminal_count_from_header(self.header()?)?
        };

        let body_prefix = {
            let _span = info_span!("parse_search_index_body_prefix").entered();
            self.parse_body_prefix()?
        };
        let mut cursor = body_prefix.cursor;
        let segments = {
            let _span = info_span!("parse_search_index_segments").entered();
            self.parse_segments(&mut cursor, body_prefix.segment_count, mode)?
        };
        let extension_suffixes = {
            let _span = info_span!("parse_search_index_extensions").entered();
            self.parse_extension_suffixes(&mut cursor, body_prefix.extension_count, mode)?
        };
        let trigrams = {
            let _span = info_span!("parse_search_index_trigrams").entered();
            self.parse_trigrams(&mut cursor, body_prefix.trigram_count)?
        };

        let layout = {
            let _span = info_span!("compute_search_index_layout").entered();
            compute_search_index_layout(
                cursor,
                SearchIndexLayoutCounts {
                    segments: body_prefix.segment_count,
                    extensions: body_prefix.extension_count,
                    trigrams: body_prefix.trigram_count,
                    path_nodes: body_prefix.path_node_count,
                    terminals: terminal_count,
                    posting_row_ids: body_prefix.posting_row_id_count,
                    extension_posting_row_ids: body_prefix.extension_posting_row_id_count,
                    trigram_posting_segment_ids: body_prefix.trigram_posting_segment_id_count,
                },
            )?
        };
        let path_node_offset = layout.path_node;
        let terminal_offset = layout.terminal;
        let posting_range_offset = layout.posting_range;
        let posting_row_id_offset = layout.posting_row_id;
        let extension_posting_range_offset = layout.extension_posting_range;
        let extension_posting_row_id_offset = layout.extension_posting_row_id;
        let trigram_posting_range_offset = layout.trigram_posting_range;
        let trigram_posting_segment_id_offset = layout.trigram_posting_segment_id;
        let end = layout.end;
        if end != self.bytes.len() {
            bail!(
                "Corrupt search index: expected {} bytes total from encoded section counts, found {}",
                end,
                self.bytes.len()
            );
        }

        if mode.should_validate() {
            validate_parsed_tables(self.bytes, &body_prefix, terminal_count, &segments, &layout)?;
        }

        Ok(ParsedSearchIndex {
            bytes: self.bytes,
            trie_bytes: body_prefix.trie_bytes,
            segments,
            extension_suffixes,
            trigrams,
            path_node_offset,
            terminal_offset,
            terminal_count,
            posting_range_offset,
            posting_row_id_offset,
            posting_row_id_count: body_prefix.posting_row_id_count,
            extension_posting_range_offset,
            extension_posting_row_id_offset,
            extension_posting_row_id_count: body_prefix.extension_posting_row_id_count,
            trigram_posting_range_offset,
            trigram_posting_segment_id_offset,
            trigram_posting_segment_id_count: body_prefix.trigram_posting_segment_id_count,
        })
    }

    fn parse_body_prefix(&self) -> eyre::Result<SearchIndexBodyPrefix<'a>> {
        let mut cursor = SEARCH_INDEX_HEADER_LEN;
        let segment_count = read_u32(self.bytes, &mut cursor) as usize;
        let extension_count = read_u32(self.bytes, &mut cursor) as usize;
        let trigram_count = read_u32(self.bytes, &mut cursor) as usize;
        let path_node_count = read_u32(self.bytes, &mut cursor) as usize;
        let trie_len = read_u32(self.bytes, &mut cursor) as usize;
        let posting_row_id_count = read_u32(self.bytes, &mut cursor) as usize;
        let extension_posting_row_id_count = read_u32(self.bytes, &mut cursor) as usize;
        let trigram_posting_segment_id_count = read_u32(self.bytes, &mut cursor) as usize;

        let trie_end = cursor + trie_len;
        if trie_end > self.bytes.len() {
            bail!(
                "Corrupt search index: truncated segment trie payload (expected {} bytes, have {})",
                trie_len,
                self.bytes.len().saturating_sub(cursor)
            );
        }

        let trie_bytes = &self.bytes[cursor..trie_end];
        cursor = trie_end;

        Ok(SearchIndexBodyPrefix {
            cursor,
            segment_count,
            extension_count,
            trigram_count,
            path_node_count,
            posting_row_id_count,
            extension_posting_row_id_count,
            trigram_posting_segment_id_count,
            trie_bytes,
        })
    }

    fn parse_segments(
        &self,
        cursor: &mut usize,
        segment_count: usize,
        mode: ParseMode,
    ) -> eyre::Result<Arc<[SearchIndexPathSegmentView<'a>]>> {
        let mut segments = Vec::with_capacity(segment_count);
        for segment_index in 0..segment_count {
            if self.bytes.len() < *cursor + SEARCH_INDEX_SEGMENT_LEN_PREFIX {
                bail!(
                    "Corrupt search index: truncated segment header at segment {}",
                    segment_index
                );
            }

            let display_len = read_u32(self.bytes, cursor) as usize;
            let normalized_len = read_u32(self.bytes, cursor) as usize;
            let display_end = *cursor + display_len;
            let normalized_end = display_end + normalized_len;
            if normalized_end > self.bytes.len() {
                bail!(
                    "Corrupt search index: truncated segment payload at segment {}",
                    segment_index
                );
            }

            let display_bytes = &self.bytes[*cursor..display_end];
            if mode.should_validate() {
                std::str::from_utf8(display_bytes).wrap_err_with(|| {
                    format!("Invalid UTF-8 display segment payload at segment {segment_index}")
                })?;
            }
            let normalized_bytes = &self.bytes[display_end..normalized_end];
            let normalized = if mode.should_validate() {
                std::str::from_utf8(normalized_bytes).wrap_err_with(|| {
                    format!("Invalid UTF-8 normalized segment payload at segment {segment_index}")
                })?
            } else {
                // SAFETY: trusted query parsing is only used for search indexes
                // written by this binary, which serializes normalized path
                // segments from valid Rust `str` values.
                unsafe { std::str::from_utf8_unchecked(normalized_bytes) }
            };
            segments.push(SearchIndexPathSegmentView {
                display_bytes,
                normalized,
            });
            *cursor = normalized_end;
        }

        Ok(Arc::<[SearchIndexPathSegmentView<'a>]>::from(segments))
    }

    fn parse_extension_suffixes(
        &self,
        cursor: &mut usize,
        extension_count: usize,
        mode: ParseMode,
    ) -> eyre::Result<Arc<[SearchIndexExtensionSuffixView<'a>]>> {
        let mut extension_suffixes = Vec::with_capacity(extension_count);
        for extension_index in 0..extension_count {
            if self.bytes.len() < *cursor + SEARCH_INDEX_EXTENSION_SUFFIX_LEN_PREFIX {
                bail!(
                    "Corrupt search index: truncated extension suffix header at index {}",
                    extension_index
                );
            }

            let normalized_len = read_u32(self.bytes, cursor) as usize;
            let normalized_end = *cursor + normalized_len;
            if normalized_end > self.bytes.len() {
                bail!(
                    "Corrupt search index: truncated extension suffix payload at index {}",
                    extension_index
                );
            }

            let normalized_bytes = &self.bytes[*cursor..normalized_end];
            let normalized_suffix = if mode.should_validate() {
                std::str::from_utf8(normalized_bytes).wrap_err_with(|| {
                    format!(
                        "Invalid UTF-8 normalized extension suffix payload at index {extension_index}"
                    )
                })?
            } else {
                // SAFETY: trusted query parsing is only used for search indexes
                // written by this binary, which serializes normalized suffixes
                // from valid Rust `str` values.
                unsafe { std::str::from_utf8_unchecked(normalized_bytes) }
            };

            extension_suffixes.push(SearchIndexExtensionSuffixView { normalized_suffix });
            *cursor = normalized_end;
        }

        Ok(Arc::<[SearchIndexExtensionSuffixView<'a>]>::from(
            extension_suffixes,
        ))
    }

    fn parse_trigrams(
        &self,
        cursor: &mut usize,
        trigram_count: usize,
    ) -> eyre::Result<Arc<[[u8; SEARCH_INDEX_TRIGRAM_LEN]]>> {
        let mut trigrams = Vec::with_capacity(trigram_count);
        for trigram_index in 0..trigram_count {
            if self.bytes.len() < *cursor + SEARCH_INDEX_TRIGRAM_LEN {
                bail!(
                    "Corrupt search index: truncated trigram payload at index {}",
                    trigram_index
                );
            }

            let trigram = [
                self.bytes[*cursor],
                self.bytes[*cursor + 1],
                self.bytes[*cursor + 2],
            ];
            trigrams.push(trigram);
            *cursor += SEARCH_INDEX_TRIGRAM_LEN;
        }

        Ok(Arc::<[[u8; SEARCH_INDEX_TRIGRAM_LEN]]>::from(trigrams))
    }
}

fn terminal_count_from_header(header: SearchIndexHeader) -> eyre::Result<usize> {
    usize::try_from(header.node_count).wrap_err_with(|| {
        format!(
            "Search index terminal count {} does not fit into usize",
            header.node_count
        )
    })
}

fn validate_parsed_tables(
    bytes: &[u8],
    body_prefix: &SearchIndexBodyPrefix<'_>,
    terminal_count: usize,
    segments: &[SearchIndexPathSegmentView<'_>],
    layout: &SearchIndexLayout,
) -> eyre::Result<()> {
    info_span!("validate_search_index_path_nodes").in_scope(|| {
        validate_path_nodes(
            bytes,
            layout.path_node,
            body_prefix.path_node_count,
            segments,
        )?;
        Ok::<(), eyre::Report>(())
    })?;
    info_span!("validate_search_index_terminals").in_scope(|| {
        validate_terminal_rows(
            bytes,
            layout.terminal,
            terminal_count,
            body_prefix.path_node_count,
        )?;
        Ok::<(), eyre::Report>(())
    })?;
    info_span!("validate_search_index_postings").in_scope(|| {
        validate_posting_table(
            bytes,
            layout.posting_range,
            layout.posting_row_id,
            body_prefix.segment_count,
            body_prefix.posting_row_id_count,
            terminal_count,
            "segment",
        )?;
        Ok::<(), eyre::Report>(())
    })?;
    info_span!("validate_search_index_extension_postings").in_scope(|| {
        validate_posting_table(
            bytes,
            layout.extension_posting_range,
            layout.extension_posting_row_id,
            body_prefix.extension_count,
            body_prefix.extension_posting_row_id_count,
            terminal_count,
            "extension suffix",
        )?;
        Ok::<(), eyre::Report>(())
    })?;
    info_span!("validate_search_index_trigram_postings").in_scope(|| {
        validate_posting_table(
            bytes,
            layout.trigram_posting_range,
            layout.trigram_posting_segment_id,
            body_prefix.trigram_count,
            body_prefix.trigram_posting_segment_id_count,
            body_prefix.segment_count,
            "trigram",
        )?;
        Ok::<(), eyre::Report>(())
    })?;

    Ok(())
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ParseMode {
    Validated,
    Trusted,
}

impl ParseMode {
    fn should_validate(self) -> bool {
        matches!(self, Self::Validated)
    }
}

impl<'a> ParsedSearchIndex<'a> {
    #[must_use]
    pub fn trie(&self) -> &'a ZeroTriePerfectHash<[u8]> {
        ZeroTriePerfectHash::from_bytes(self.trie_bytes)
    }

    #[must_use]
    pub fn segments(&self) -> &[SearchIndexPathSegmentView<'a>] {
        &self.segments
    }

    /// # Errors
    ///
    /// Returns an error if `segment_id` is out of bounds.
    pub fn segment(&self, segment_id: u32) -> eyre::Result<SearchIndexPathSegmentView<'a>> {
        let segment_id = segment_id as usize;
        self.segments.get(segment_id).copied().ok_or_else(|| {
            eyre::eyre!(
                "Corrupt search index: requested segment {} but index contains {} segments",
                segment_id,
                self.segments.len()
            )
        })
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.terminal_count
    }

    #[must_use]
    pub fn row_views(&self) -> SearchIndexRowIter<'a> {
        SearchIndexRowIter {
            parsed: self.clone(),
            next_terminal_offset: self.terminal_offset,
            remaining_rows: self.terminal_count,
            row_index: 0,
        }
    }

    /// # Errors
    ///
    /// Returns an error if `row_index` is out of bounds.
    pub fn row_view(&self, row_index: usize) -> eyre::Result<SearchIndexPathRowView<'a>> {
        if row_index >= self.terminal_count {
            bail!(
                "Corrupt search index: requested terminal row {} but index contains {} rows",
                row_index,
                self.terminal_count
            );
        }

        let terminal_offset = self.terminal_offset + row_index * SEARCH_INDEX_TERMINAL_LEN;
        let terminal_node_index = read_u32_at(self.bytes, terminal_offset);
        let has_deleted_entries = self.bytes[terminal_offset + 4] != 0;

        Ok(SearchIndexPathRowView {
            parsed: self.clone(),
            terminal_node_index,
            has_deleted_entries,
        })
    }

    /// # Errors
    ///
    /// Returns an error if `segment_id` is out of bounds.
    pub fn postings(&self, segment_id: u32) -> eyre::Result<SearchIndexPostingIter<'a>> {
        let segment_id = segment_id as usize;
        if segment_id >= self.segments.len() {
            bail!(
                "Corrupt search index: requested segment {} but index contains {} segments",
                segment_id,
                self.segments.len()
            );
        }

        self.posting_iter(
            self.posting_range_offset,
            self.posting_row_id_offset,
            self.posting_row_id_count,
            segment_id,
            "segment",
        )
    }

    /// # Errors
    ///
    /// Returns an error if the extension posting range is malformed.
    pub fn extension_postings(
        &self,
        normalized_suffix: &str,
    ) -> eyre::Result<Option<SearchIndexPostingIter<'a>>> {
        let Ok(extension_index) = self
            .extension_suffixes
            .binary_search_by_key(&normalized_suffix, |entry| entry.normalized_suffix)
        else {
            return Ok(None);
        };

        Ok(Some(self.posting_iter(
            self.extension_posting_range_offset,
            self.extension_posting_row_id_offset,
            self.extension_posting_row_id_count,
            extension_index,
            "extension suffix",
        )?))
    }

    /// # Errors
    ///
    /// Returns an error if the trigram posting range is malformed.
    pub fn trigram_postings(
        &self,
        trigram: [u8; SEARCH_INDEX_TRIGRAM_LEN],
    ) -> eyre::Result<Option<SearchIndexPostingIter<'a>>> {
        let Ok(trigram_index) = self.trigrams.binary_search(&trigram) else {
            return Ok(None);
        };

        Ok(Some(self.posting_iter(
            self.trigram_posting_range_offset,
            self.trigram_posting_segment_id_offset,
            self.trigram_posting_segment_id_count,
            trigram_index,
            "trigram",
        )?))
    }

    fn posting_iter(
        &self,
        range_table_offset: usize,
        posting_row_id_offset: usize,
        posting_row_id_count: usize,
        entry_index: usize,
        entry_kind: &str,
    ) -> eyre::Result<SearchIndexPostingIter<'a>> {
        let range_offset = range_table_offset + entry_index * SEARCH_INDEX_POSTING_RANGE_LEN;
        let posting_start = read_u32_at(self.bytes, range_offset) as usize;
        let posting_len = read_u32_at(self.bytes, range_offset + 4) as usize;
        let posting_end = posting_start + posting_len;
        if posting_end > posting_row_id_count {
            bail!(
                "Corrupt search index: {} {} posting range {}..{} exceeds posting table length {}",
                entry_kind,
                entry_index,
                posting_start,
                posting_end,
                posting_row_id_count
            );
        }

        Ok(SearchIndexPostingIter {
            bytes: self.bytes,
            next_offset: posting_row_id_offset + posting_start * SEARCH_INDEX_POSTING_ROW_LEN,
            remaining_rows: posting_len,
        })
    }

    fn node_segment(&self, node_index: u32) -> SearchIndexPathSegmentView<'a> {
        let node_offset = self.path_node_offset + node_index as usize * SEARCH_INDEX_NODE_LEN;
        let segment_id = read_u32_at(self.bytes, node_offset) as usize;
        self.segments[segment_id]
    }

    fn node_parent(&self, node_index: u32) -> Option<u32> {
        let node_offset = self.path_node_offset + node_index as usize * SEARCH_INDEX_NODE_LEN;
        let parent = read_u32_at(self.bytes, node_offset + 4);
        (parent != NO_PARENT_NODE).then_some(parent)
    }
}

impl<'a> Iterator for SearchIndexPathSegmentIter<'a> {
    type Item = SearchIndexPathSegmentView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let node_index = self.next_node_index?;
        let segment = self.parsed.node_segment(node_index);
        self.next_node_index = self.parsed.node_parent(node_index);
        Some(segment)
    }
}

impl<'a> Iterator for SearchIndexRowIter<'a> {
    type Item = eyre::Result<SearchIndexPathRowView<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_rows == 0 {
            return None;
        }

        let row_index = self.row_index;
        self.row_index += 1;
        self.remaining_rows -= 1;

        if self.parsed.bytes.len() < self.next_terminal_offset + SEARCH_INDEX_TERMINAL_LEN {
            return Some(Err(eyre::eyre!(
                "Corrupt search index: truncated terminal row at row {}",
                row_index
            )));
        }

        let terminal_node_index = read_u32_at(self.parsed.bytes, self.next_terminal_offset);
        let has_deleted_entries = self.parsed.bytes[self.next_terminal_offset + 4] != 0;
        self.next_terminal_offset += SEARCH_INDEX_TERMINAL_LEN;

        Some(Ok(SearchIndexPathRowView {
            parsed: self.parsed.clone(),
            terminal_node_index,
            has_deleted_entries,
        }))
    }
}

impl Iterator for SearchIndexPostingIter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_rows == 0 {
            return None;
        }

        let row_id = read_u32_at(self.bytes, self.next_offset);
        self.next_offset += SEARCH_INDEX_POSTING_ROW_LEN;
        self.remaining_rows -= 1;
        Some(row_id)
    }
}

impl SearchIndexBytesMut {
    #[must_use]
    pub fn new(header: SearchIndexHeader) -> Self {
        Self {
            header,
            rows: Vec::new(),
        }
    }

    /// # Errors
    ///
    /// Returns an error if the row cannot be buffered for later serialization.
    pub fn push_row(&mut self, row: &SearchIndexPathRow) -> eyre::Result<()> {
        self.rows.push(row.clone());
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if any row cannot be buffered for later serialization.
    pub fn extend_rows<'b>(
        &mut self,
        rows: impl IntoIterator<Item = &'b SearchIndexPathRow>,
    ) -> eyre::Result<()> {
        for row in rows {
            self.push_row(row)?;
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the provided header version is unsupported or any row
    /// cannot be buffered for later serialization.
    pub fn from_rows(header: SearchIndexHeader, rows: &[SearchIndexPathRow]) -> eyre::Result<Self> {
        if header.version != SEARCH_INDEX_VERSION {
            bail!(
                "Cannot build search index bytes with unsupported version {} (expected {})",
                header.version,
                SEARCH_INDEX_VERSION
            );
        }

        let mut bytes = Self::new(header);
        bytes.extend_rows(rows.iter())?;
        Ok(bytes)
    }

    #[must_use]
    pub fn header(&self) -> SearchIndexHeader {
        self.header
    }

    /// # Errors
    ///
    /// Returns an error if the buffered rows cannot be serialized into the
    /// search-index byte format.
    pub fn into_inner(self) -> eyre::Result<Vec<u8>> {
        serialize_search_index(self.header, &self.rows)
    }

    /// Write the search index to `output_path` atomically via a temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error if the output file cannot be created or written,
    /// or if the temporary file cannot be renamed into place.
    pub fn write_to_path(self, output_path: impl AsRef<Path>) -> eyre::Result<()> {
        let output_path = output_path.as_ref();
        let temp_path = output_path.with_extension("mft_search_index.tmp");
        let bytes = self.into_inner()?;

        let file = std::fs::File::create(&temp_path).wrap_err_with(|| {
            format!(
                "Failed creating temporary search index file {}",
                temp_path.display()
            )
        })?;
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(&bytes)?;
        writer.flush()?;

        std::fs::rename(&temp_path, output_path).wrap_err_with(|| {
            format!(
                "Failed atomically renaming search index file {} -> {}",
                temp_path.display(),
                output_path.display()
            )
        })?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct SearchIndexNodeRecord {
    segment_id: u32,
    parent_node_index: u32,
}

#[derive(Debug, Clone, Copy)]
struct SearchIndexLayout {
    path_node: usize,
    terminal: usize,
    posting_range: usize,
    posting_row_id: usize,
    extension_posting_range: usize,
    extension_posting_row_id: usize,
    trigram_posting_range: usize,
    trigram_posting_segment_id: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy)]
struct SearchIndexLayoutCounts {
    segments: usize,
    extensions: usize,
    trigrams: usize,
    path_nodes: usize,
    terminals: usize,
    posting_row_ids: usize,
    extension_posting_row_ids: usize,
    trigram_posting_segment_ids: usize,
}

#[derive(Debug)]
struct SearchIndexSerializationTables {
    segment_entries: Vec<(String, String)>,
    postings_by_segment: Vec<Vec<u32>>,
    extension_postings_by_suffix: BTreeMap<String, Vec<u32>>,
    trigram_postings_by_trigram: BTreeMap<[u8; SEARCH_INDEX_TRIGRAM_LEN], Vec<u32>>,
    trie_bytes: Vec<u8>,
    path_nodes: Vec<SearchIndexNodeRecord>,
    terminals: Vec<(u32, bool)>,
}

fn serialize_search_index(
    header: SearchIndexHeader,
    rows: &[SearchIndexPathRow],
) -> eyre::Result<Vec<u8>> {
    let tables = collect_search_index_tables(rows)?;
    serialize_search_index_tables(header, tables, rows.len())
}

fn collect_search_index_tables(
    rows: &[SearchIndexPathRow],
) -> eyre::Result<SearchIndexSerializationTables> {
    let mut segment_ids_by_display = HashMap::<String, u32>::new();
    let mut segment_entries = Vec::<(String, String)>::new();
    let mut postings_by_segment = Vec::<Vec<u32>>::new();
    let mut extension_postings_by_suffix = BTreeMap::<String, Vec<u32>>::new();
    let mut trigram_postings_by_trigram =
        BTreeMap::<[u8; SEARCH_INDEX_TRIGRAM_LEN], Vec<u32>>::new();
    let mut normalized_trie_entries = BTreeMap::<Vec<u8>, usize>::new();
    let mut path_nodes = Vec::<SearchIndexNodeRecord>::new();
    let mut terminals = Vec::<(u32, bool)>::with_capacity(rows.len());

    for row in rows {
        let row_index = u32::try_from(terminals.len())
            .wrap_err_with(|| format!("Too many terminal rows to encode ({})", terminals.len()))?;
        let mut parent_node_index = NO_PARENT_NODE;
        let mut row_segment_ids = Vec::new();
        for segment in path_segments(&row.path) {
            let segment_id = if let Some(existing_id) = segment_ids_by_display.get(segment) {
                *existing_id
            } else {
                let normalized = segment.to_lowercase();
                let segment_id = u32::try_from(segment_entries.len()).wrap_err_with(|| {
                    format!(
                        "Too many unique path segments to encode ({})",
                        segment_entries.len()
                    )
                })?;
                normalized_trie_entries
                    .entry(normalized.as_bytes().to_vec())
                    .or_insert(segment_id as usize);
                for trigram in normalized_trigrams(&normalized) {
                    trigram_postings_by_trigram
                        .entry(trigram)
                        .or_default()
                        .push(segment_id);
                }
                segment_entries.push((segment.to_owned(), normalized));
                postings_by_segment.push(Vec::new());
                segment_ids_by_display.insert(segment.to_owned(), segment_id);
                segment_id
            };

            row_segment_ids.push(segment_id);

            let node_index = u32::try_from(path_nodes.len()).wrap_err_with(|| {
                format!("Too many path nodes to encode ({})", path_nodes.len())
            })?;
            path_nodes.push(SearchIndexNodeRecord {
                segment_id,
                parent_node_index,
            });
            parent_node_index = node_index;
        }

        if parent_node_index == NO_PARENT_NODE {
            bail!("Cannot encode empty path into search index");
        }

        row_segment_ids.sort_unstable();
        row_segment_ids.dedup();
        for &segment_id in &row_segment_ids {
            postings_by_segment[segment_id as usize].push(row_index);

            let normalized = &segment_entries[segment_id as usize].1;
            if let Some(normalized_suffix) = normalized_extension_suffix(normalized) {
                extension_postings_by_suffix
                    .entry(normalized_suffix.to_owned())
                    .or_default()
                    .push(row_index);
            }
        }

        terminals.push((parent_node_index, row.has_deleted_entries));
    }

    for row_ids in extension_postings_by_suffix.values_mut() {
        row_ids.sort_unstable();
        row_ids.dedup();
    }

    let trie: ZeroTriePerfectHash<Vec<u8>> = normalized_trie_entries.into_iter().collect();
    Ok(SearchIndexSerializationTables {
        segment_entries,
        postings_by_segment,
        extension_postings_by_suffix,
        trigram_postings_by_trigram,
        trie_bytes: trie.into_store(),
        path_nodes,
        terminals,
    })
}

fn serialize_search_index_tables(
    header: SearchIndexHeader,
    tables: SearchIndexSerializationTables,
    row_count: usize,
) -> eyre::Result<Vec<u8>> {
    let SearchIndexSerializationTables {
        segment_entries,
        postings_by_segment,
        extension_postings_by_suffix,
        trigram_postings_by_trigram,
        trie_bytes,
        path_nodes,
        terminals,
    } = tables;

    let segment_count: u32 = segment_entries.len().try_into().wrap_err_with(|| {
        format!(
            "Too many unique path segments to encode ({})",
            segment_entries.len()
        )
    })?;
    let extension_count: u32 = extension_postings_by_suffix
        .len()
        .try_into()
        .wrap_err_with(|| {
            format!(
                "Too many unique extension suffixes to encode ({})",
                extension_postings_by_suffix.len()
            )
        })?;
    let trigram_count: u32 = trigram_postings_by_trigram
        .len()
        .try_into()
        .wrap_err_with(|| {
            format!(
                "Too many unique trigrams to encode ({})",
                trigram_postings_by_trigram.len()
            )
        })?;
    let path_node_count: u32 = path_nodes
        .len()
        .try_into()
        .wrap_err_with(|| format!("Too many path nodes to encode ({})", path_nodes.len()))?;
    let trie_len: u32 = trie_bytes.len().try_into().wrap_err_with(|| {
        format!(
            "Segment trie too large to encode ({} bytes)",
            trie_bytes.len()
        )
    })?;
    let posting_row_id_count: u32 = postings_by_segment
        .iter()
        .map(std::vec::Vec::len)
        .sum::<usize>()
        .try_into()
        .wrap_err("Too many segment posting row ids to encode")?;
    let extension_posting_row_id_count: u32 = extension_postings_by_suffix
        .values()
        .map(std::vec::Vec::len)
        .sum::<usize>()
        .try_into()
        .wrap_err("Too many extension posting row ids to encode")?;
    let trigram_posting_segment_id_count: u32 = trigram_postings_by_trigram
        .values()
        .map(std::vec::Vec::len)
        .sum::<usize>()
        .try_into()
        .wrap_err("Too many trigram posting segment ids to encode")?;

    let mut bytes = Vec::with_capacity(SEARCH_INDEX_HEADER_LEN + row_count * 16);
    header.extend_vec(&mut bytes);
    bytes.extend_from_slice(&segment_count.to_le_bytes());
    bytes.extend_from_slice(&extension_count.to_le_bytes());
    bytes.extend_from_slice(&trigram_count.to_le_bytes());
    bytes.extend_from_slice(&path_node_count.to_le_bytes());
    bytes.extend_from_slice(&trie_len.to_le_bytes());
    bytes.extend_from_slice(&posting_row_id_count.to_le_bytes());
    bytes.extend_from_slice(&extension_posting_row_id_count.to_le_bytes());
    bytes.extend_from_slice(&trigram_posting_segment_id_count.to_le_bytes());
    bytes.extend_from_slice(&trie_bytes);

    serialize_segment_entries(&mut bytes, &segment_entries)?;
    serialize_extension_suffix_entries(&mut bytes, &extension_postings_by_suffix)?;
    serialize_trigram_entries(&mut bytes, &trigram_postings_by_trigram);
    serialize_path_nodes(&mut bytes, &path_nodes);
    serialize_terminals(&mut bytes, &terminals);
    serialize_postings(&mut bytes, &postings_by_segment, posting_row_id_count)?;
    serialize_extension_postings(
        &mut bytes,
        &extension_postings_by_suffix,
        extension_posting_row_id_count,
    )?;
    serialize_trigram_postings(
        &mut bytes,
        &trigram_postings_by_trigram,
        trigram_posting_segment_id_count,
    )?;

    Ok(bytes)
}

fn compute_search_index_layout(
    cursor: usize,
    counts: SearchIndexLayoutCounts,
) -> eyre::Result<SearchIndexLayout> {
    let path_node_bytes_len = counts
        .path_nodes
        .checked_mul(SEARCH_INDEX_NODE_LEN)
        .ok_or_else(|| eyre::eyre!("Search index path-node table length overflow"))?;
    let terminal_bytes_len = counts
        .terminals
        .checked_mul(SEARCH_INDEX_TERMINAL_LEN)
        .ok_or_else(|| eyre::eyre!("Search index terminal table length overflow"))?;
    let posting_range_bytes_len = counts
        .segments
        .checked_mul(SEARCH_INDEX_POSTING_RANGE_LEN)
        .ok_or_else(|| eyre::eyre!("Search index posting-range table length overflow"))?;
    let posting_row_id_bytes_len = counts
        .posting_row_ids
        .checked_mul(SEARCH_INDEX_POSTING_ROW_LEN)
        .ok_or_else(|| eyre::eyre!("Search index posting row-id table length overflow"))?;
    let extension_posting_range_bytes_len = counts
        .extensions
        .checked_mul(SEARCH_INDEX_POSTING_RANGE_LEN)
        .ok_or_else(|| eyre::eyre!("Search index extension posting-range table length overflow"))?;
    let extension_posting_row_id_bytes_len = counts
        .extension_posting_row_ids
        .checked_mul(SEARCH_INDEX_POSTING_ROW_LEN)
        .ok_or_else(|| {
            eyre::eyre!("Search index extension posting row-id table length overflow")
        })?;
    let trigram_posting_range_bytes_len = counts
        .trigrams
        .checked_mul(SEARCH_INDEX_POSTING_RANGE_LEN)
        .ok_or_else(|| eyre::eyre!("Search index trigram posting-range table length overflow"))?;
    let trigram_posting_segment_id_bytes_len = counts
        .trigram_posting_segment_ids
        .checked_mul(SEARCH_INDEX_POSTING_ROW_LEN)
        .ok_or_else(|| {
            eyre::eyre!("Search index trigram posting segment-id table length overflow")
        })?;

    let path_node_offset = cursor;
    let terminal_offset = path_node_offset + path_node_bytes_len;
    let posting_range_offset = terminal_offset + terminal_bytes_len;
    let posting_row_id_offset = posting_range_offset + posting_range_bytes_len;
    let extension_posting_range_offset = posting_row_id_offset + posting_row_id_bytes_len;
    let extension_posting_row_id_offset =
        extension_posting_range_offset + extension_posting_range_bytes_len;
    let trigram_posting_range_offset =
        extension_posting_row_id_offset + extension_posting_row_id_bytes_len;
    let trigram_posting_segment_id_offset =
        trigram_posting_range_offset + trigram_posting_range_bytes_len;
    let end_offset = trigram_posting_segment_id_offset + trigram_posting_segment_id_bytes_len;

    Ok(SearchIndexLayout {
        path_node: path_node_offset,
        terminal: terminal_offset,
        posting_range: posting_range_offset,
        posting_row_id: posting_row_id_offset,
        extension_posting_range: extension_posting_range_offset,
        extension_posting_row_id: extension_posting_row_id_offset,
        trigram_posting_range: trigram_posting_range_offset,
        trigram_posting_segment_id: trigram_posting_segment_id_offset,
        end: end_offset,
    })
}

fn validate_path_nodes(
    bytes: &[u8],
    path_node_offset: usize,
    path_node_count: usize,
    segments: &[SearchIndexPathSegmentView<'_>],
) -> eyre::Result<()> {
    for node_index in 0..path_node_count {
        let node_offset = path_node_offset + node_index * SEARCH_INDEX_NODE_LEN;
        let segment_id = read_u32_at(bytes, node_offset) as usize;
        let parent_node_index = read_u32_at(bytes, node_offset + 4);

        if segment_id >= segments.len() {
            bail!(
                "Corrupt search index: path node {} references missing segment {}",
                node_index,
                segment_id
            );
        }
        if parent_node_index != NO_PARENT_NODE && parent_node_index as usize >= path_node_count {
            bail!(
                "Corrupt search index: path node {} references missing parent {}",
                node_index,
                parent_node_index
            );
        }
    }

    Ok(())
}

fn validate_terminal_rows(
    bytes: &[u8],
    terminal_offset: usize,
    terminal_count: usize,
    path_node_count: usize,
) -> eyre::Result<()> {
    for row_index in 0..terminal_count {
        let terminal_row_offset = terminal_offset + row_index * SEARCH_INDEX_TERMINAL_LEN;
        let terminal_node_index = read_u32_at(bytes, terminal_row_offset) as usize;
        if terminal_node_index >= path_node_count {
            bail!(
                "Corrupt search index: terminal row {} references missing path node {}",
                row_index,
                terminal_node_index
            );
        }
    }

    Ok(())
}

fn validate_posting_table(
    bytes: &[u8],
    posting_range_offset: usize,
    posting_row_id_offset: usize,
    entry_count: usize,
    posting_row_id_count: usize,
    max_posting_id_exclusive: usize,
    entry_kind: &str,
) -> eyre::Result<()> {
    for entry_index in 0..entry_count {
        let range_offset = posting_range_offset + entry_index * SEARCH_INDEX_POSTING_RANGE_LEN;
        let posting_start = read_u32_at(bytes, range_offset) as usize;
        let posting_len = read_u32_at(bytes, range_offset + 4) as usize;
        let posting_end = posting_start + posting_len;
        if posting_end > posting_row_id_count {
            bail!(
                "Corrupt search index: {} {} posting range {}..{} exceeds posting table length {}",
                entry_kind,
                entry_index,
                posting_start,
                posting_end,
                posting_row_id_count
            );
        }

        for posting_index in posting_start..posting_end {
            let row_id_offset =
                posting_row_id_offset + posting_index * SEARCH_INDEX_POSTING_ROW_LEN;
            let row_id = read_u32_at(bytes, row_id_offset) as usize;
            if row_id >= max_posting_id_exclusive {
                bail!(
                    "Corrupt search index: {} {} posting references missing target id {}",
                    entry_kind,
                    entry_index,
                    row_id
                );
            }
        }
    }

    Ok(())
}

fn normalized_extension_suffix(segment: &str) -> Option<&str> {
    let dot_index = segment.rfind('.')?;
    (dot_index + 1 < segment.len()).then_some(&segment[dot_index..])
}

fn serialize_segment_entries(
    bytes: &mut Vec<u8>,
    segment_entries: &[(String, String)],
) -> eyre::Result<()> {
    for (display, normalized) in segment_entries {
        let display_bytes = display.as_bytes();
        let normalized_bytes = normalized.as_bytes();
        let display_len: u32 = display_bytes.len().try_into().wrap_err_with(|| {
            format!(
                "Segment display text too long to encode ({} bytes)",
                display_bytes.len()
            )
        })?;
        let normalized_len: u32 = normalized_bytes.len().try_into().wrap_err_with(|| {
            format!(
                "Segment normalized text too long to encode ({} bytes)",
                normalized_bytes.len()
            )
        })?;

        bytes.extend_from_slice(&display_len.to_le_bytes());
        bytes.extend_from_slice(&normalized_len.to_le_bytes());
        bytes.extend_from_slice(display_bytes);
        bytes.extend_from_slice(normalized_bytes);
    }

    Ok(())
}

fn serialize_extension_suffix_entries(
    bytes: &mut Vec<u8>,
    extension_postings_by_suffix: &BTreeMap<String, Vec<u32>>,
) -> eyre::Result<()> {
    for normalized_suffix in extension_postings_by_suffix.keys() {
        let normalized_bytes = normalized_suffix.as_bytes();
        let normalized_len: u32 = normalized_bytes.len().try_into().wrap_err_with(|| {
            format!(
                "Extension suffix text too long to encode ({} bytes)",
                normalized_bytes.len()
            )
        })?;

        bytes.extend_from_slice(&normalized_len.to_le_bytes());
        bytes.extend_from_slice(normalized_bytes);
    }

    Ok(())
}

fn serialize_trigram_entries(
    bytes: &mut Vec<u8>,
    trigram_postings_by_trigram: &BTreeMap<[u8; SEARCH_INDEX_TRIGRAM_LEN], Vec<u32>>,
) {
    for trigram in trigram_postings_by_trigram.keys() {
        bytes.extend_from_slice(trigram);
    }
}

fn serialize_path_nodes(bytes: &mut Vec<u8>, path_nodes: &[SearchIndexNodeRecord]) {
    for node in path_nodes {
        bytes.extend_from_slice(&node.segment_id.to_le_bytes());
        bytes.extend_from_slice(&node.parent_node_index.to_le_bytes());
    }
}

fn serialize_terminals(bytes: &mut Vec<u8>, terminals: &[(u32, bool)]) {
    for &(terminal_node_index, has_deleted_entries) in terminals {
        bytes.extend_from_slice(&terminal_node_index.to_le_bytes());
        bytes.extend_from_slice(&[u8::from(has_deleted_entries)]);
    }
}

fn serialize_postings(
    bytes: &mut Vec<u8>,
    postings_by_segment: &[Vec<u32>],
    posting_row_id_count: u32,
) -> eyre::Result<()> {
    let mut posting_row_ids = Vec::with_capacity(posting_row_id_count as usize);
    for postings in postings_by_segment {
        let posting_start: u32 = posting_row_ids.len().try_into().wrap_err_with(|| {
            format!(
                "Too many segment posting row ids to encode ({})",
                posting_row_ids.len()
            )
        })?;
        let posting_len: u32 = postings.len().try_into().wrap_err_with(|| {
            format!(
                "Too many postings for one segment to encode ({})",
                postings.len()
            )
        })?;

        bytes.extend_from_slice(&posting_start.to_le_bytes());
        bytes.extend_from_slice(&posting_len.to_le_bytes());
        posting_row_ids.extend(postings.iter().copied());
    }

    for row_id in posting_row_ids {
        bytes.extend_from_slice(&row_id.to_le_bytes());
    }

    Ok(())
}

fn serialize_extension_postings(
    bytes: &mut Vec<u8>,
    extension_postings_by_suffix: &BTreeMap<String, Vec<u32>>,
    extension_posting_row_id_count: u32,
) -> eyre::Result<()> {
    let mut posting_row_ids = Vec::with_capacity(extension_posting_row_id_count as usize);
    for postings in extension_postings_by_suffix.values() {
        let posting_start: u32 = posting_row_ids.len().try_into().wrap_err_with(|| {
            format!(
                "Too many extension posting row ids to encode ({})",
                posting_row_ids.len()
            )
        })?;
        let posting_len: u32 = postings.len().try_into().wrap_err_with(|| {
            format!(
                "Too many postings for one extension suffix to encode ({})",
                postings.len()
            )
        })?;

        bytes.extend_from_slice(&posting_start.to_le_bytes());
        bytes.extend_from_slice(&posting_len.to_le_bytes());
        posting_row_ids.extend(postings.iter().copied());
    }

    for row_id in posting_row_ids {
        bytes.extend_from_slice(&row_id.to_le_bytes());
    }

    Ok(())
}

fn serialize_trigram_postings(
    bytes: &mut Vec<u8>,
    trigram_postings_by_trigram: &BTreeMap<[u8; SEARCH_INDEX_TRIGRAM_LEN], Vec<u32>>,
    trigram_posting_segment_id_count: u32,
) -> eyre::Result<()> {
    let mut posting_segment_ids = Vec::with_capacity(trigram_posting_segment_id_count as usize);
    for postings in trigram_postings_by_trigram.values() {
        let posting_start: u32 = posting_segment_ids.len().try_into().wrap_err_with(|| {
            format!(
                "Too many trigram posting segment ids to encode ({})",
                posting_segment_ids.len()
            )
        })?;
        let posting_len: u32 = postings.len().try_into().wrap_err_with(|| {
            format!(
                "Too many postings for one trigram to encode ({})",
                postings.len()
            )
        })?;

        bytes.extend_from_slice(&posting_start.to_le_bytes());
        bytes.extend_from_slice(&posting_len.to_le_bytes());
        posting_segment_ids.extend(postings.iter().copied());
    }

    for segment_id in posting_segment_ids {
        bytes.extend_from_slice(&segment_id.to_le_bytes());
    }

    Ok(())
}

fn normalized_trigrams(normalized: &str) -> Vec<[u8; SEARCH_INDEX_TRIGRAM_LEN]> {
    let mut trigrams = normalized
        .as_bytes()
        .windows(SEARCH_INDEX_TRIGRAM_LEN)
        .map(|window| [window[0], window[1], window[2]])
        .collect::<Vec<_>>();
    trigrams.sort_unstable();
    trigrams.dedup();
    trigrams
}

fn path_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> u16 {
    let value = read_u16_at(bytes, *cursor);
    *cursor += 2;
    value
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> u32 {
    let value = read_u32_at(bytes, *cursor);
    *cursor += 4;
    value
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> u64 {
    let value = read_u64_at(bytes, *cursor);
    *cursor += 8;
    value
}

fn read_u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::ParsedSearchIndex;
    use super::SearchIndexBytes;
    use super::SearchIndexBytesMut;
    use crate::search_index::format::SEARCH_INDEX_VERSION;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;

    const VIRTUAL_SNAPSHOT_TEST_PATH: &str = "Q:\\__TEAMY_MFT_VIRTUAL_SNAPSHOT_FIXTURE__\\a.txt";

    #[test]
    fn segment_dictionary_and_parent_chain_roundtrip() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\src\\target\\foo.txt"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\target\\bar.txt"),
                has_deleted_entries: true,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let search_index_bytes = SearchIndexBytes::new(&bytes);
        let parsed = search_index_bytes.parse()?;
        let trie = parsed.trie();

        assert!(trie.get("target").is_some());
        assert!(trie.get("src").is_some());
        assert!(trie.get("pkg").is_some());

        let views = parsed.row_views().collect::<eyre::Result<Vec<_>>>()?;
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].path(), "C:\\src\\target\\foo.txt");
        assert_eq!(
            views[0]
                .segment_views()
                .map(|segment| segment.normalized)
                .collect::<Vec<_>>(),
            vec!["foo.txt", "target", "src", "c:"]
        );
        assert_eq!(views[1].path(), "C:\\pkg\\target\\bar.txt");
        assert!(views[1].has_deleted_entries);
        assert_eq!(
            parsed
                .postings(segment_id_for(&parsed, "target")?)?
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(
            parsed
                .postings(segment_id_for(&parsed, "src")?)?
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(
            parsed
                .postings(segment_id_for(&parsed, "pkg")?)?
                .collect::<Vec<_>>(),
            vec![1]
        );

        Ok(())
    }

    #[test]
    fn snapshot_small_index_bytes_v6_roundtrips() -> eyre::Result<()> {
        let rows = vec![SearchIndexPathRow {
            path: String::from(VIRTUAL_SNAPSHOT_TEST_PATH),
            has_deleted_entries: false,
        }];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 7, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;

        let search_index_bytes = SearchIndexBytes::new(&bytes);
        assert_eq!(search_index_bytes.version()?, SEARCH_INDEX_VERSION);

        let parsed_rows = search_index_bytes
            .row_views()?
            .map(|row| row.map(|view| view.path()))
            .collect::<eyre::Result<Vec<_>>>()?;
        assert_eq!(parsed_rows, vec![String::from(VIRTUAL_SNAPSHOT_TEST_PATH)]);

        assert!(
            search_index_bytes
                .trie()?
                .get("__teamy_mft_virtual_snapshot_fixture__")
                .is_some()
        );
        assert!(search_index_bytes.trie()?.get("a.txt").is_some());

        let parsed = search_index_bytes.parse_trusted_for_query()?;
        assert_eq!(
            parsed
                .extension_postings(".txt")?
                .map(|iter| iter.collect::<Vec<_>>()),
            Some(vec![0])
        );

        Ok(())
    }

    #[test]
    fn trusted_query_parse_matches_validated_parse_for_well_formed_index() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\src\\flower.jar"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\trees.zip"),
                has_deleted_entries: true,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let search_index_bytes = SearchIndexBytes::new(&bytes);

        let validated = search_index_bytes.parse()?;
        let trusted = search_index_bytes.parse_trusted_for_query()?;

        assert_eq!(validated.row_count(), trusted.row_count());
        assert_eq!(validated.segments().len(), trusted.segments().len());
        assert_eq!(
            validated
                .extension_postings(".jar")?
                .map(|iter| iter.collect::<Vec<_>>()),
            trusted
                .extension_postings(".jar")?
                .map(|iter| iter.collect::<Vec<_>>())
        );
        assert_eq!(
            validated
                .row_views()
                .map(|row| row.map(|view| view.path()))
                .collect::<eyre::Result<Vec<_>>>()?,
            trusted
                .row_views()
                .map(|row| row.map(|view| view.path()))
                .collect::<eyre::Result<Vec<_>>>()?
        );

        Ok(())
    }

    #[test]
    fn extension_postings_group_segments_by_normalized_suffix() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\src\\flower.jar"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\trees.jar"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\pkg\\notes.txt"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let parsed = SearchIndexBytes::new(&bytes).parse_trusted_for_query()?;

        assert_eq!(
            parsed
                .extension_postings(".jar")?
                .map(|iter| iter.collect::<Vec<_>>()),
            Some(vec![0, 1])
        );
        assert_eq!(
            parsed
                .extension_postings(".txt")?
                .map(|iter| iter.collect::<Vec<_>>()),
            Some(vec![2])
        );
        assert_eq!(
            parsed
                .extension_postings(".zip")?
                .map(|iter| iter.collect::<Vec<_>>()),
            None
        );

        Ok(())
    }

    #[test]
    fn trigram_postings_group_segments_by_normalized_trigram() -> eyre::Result<()> {
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
        let parsed = SearchIndexBytes::new(&bytes).parse_trusted_for_query()?;

        assert_eq!(
            parsed
                .trigram_postings(*b"flo")?
                .map(|iter| iter.collect::<Vec<_>>()),
            Some(vec![
                segment_id_for(&parsed, "flower.jar")?,
                segment_id_for(&parsed, "flowchart.txt")?
            ])
        );
        assert_eq!(
            parsed
                .trigram_postings(*b"owe")?
                .map(|iter| iter.collect::<Vec<_>>()),
            Some(vec![segment_id_for(&parsed, "flower.jar")?])
        );
        assert_eq!(
            parsed
                .trigram_postings(*b"zip")?
                .map(|iter| iter.collect::<Vec<_>>()),
            Some(vec![segment_id_for(&parsed, "trees.zip")?])
        );
        assert!(parsed.trigram_postings(*b"xyz")?.is_none());

        Ok(())
    }

    fn segment_id_for(parsed: &ParsedSearchIndex<'_>, normalized: &str) -> eyre::Result<u32> {
        parsed
            .segments()
            .iter()
            .position(|segment| segment.normalized == normalized)
            .map(|segment_id| segment_id as u32)
            .ok_or_else(|| eyre::eyre!("missing segment {normalized}"))
    }
}
