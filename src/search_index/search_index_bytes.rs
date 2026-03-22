use crate::search_index::format::SEARCH_INDEX_HEADER_LEN;
use crate::search_index::format::SEARCH_INDEX_MAGIC;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use eyre::Context;
use eyre::bail;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use zerotrie::ZeroTriePerfectHash;

const SEARCH_INDEX_BODY_PREFIX_LEN: usize = 4 + 4 + 4 + 4;
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
    pub display: &'a str,
    pub normalized: &'a str,
}

#[derive(Debug, Clone)]
pub struct ParsedSearchIndex<'a> {
    bytes: &'a [u8],
    trie_bytes: &'a [u8],
    segments: Arc<[SearchIndexPathSegmentView<'a>]>,
    path_node_offset: usize,
    terminal_offset: usize,
    terminal_count: usize,
    posting_range_offset: usize,
    posting_row_id_offset: usize,
    posting_row_id_count: usize,
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
        let mut segments = self
            .segment_views()
            .map(|segment| segment.display)
            .collect::<Vec<_>>();
        segments.reverse();

        let path_len = segments.iter().map(|segment| segment.len()).sum::<usize>()
            + segments.len().saturating_sub(1);
        let mut path = String::with_capacity(path_len);
        for (index, segment) in segments.into_iter().enumerate() {
            if index > 0 {
                path.push('\\');
            }
            path.push_str(segment);
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
        self.parse_body()
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
        Ok(self.parse_body()?.row_views())
    }

    /// # Errors
    ///
    /// Returns an error if the index body cannot be parsed or `row_index` is
    /// out of bounds.
    pub fn row_view(&self, row_index: usize) -> eyre::Result<SearchIndexPathRowView<'a>> {
        self.parse_body()?.row_view(row_index)
    }

    /// # Errors
    ///
    /// Returns an error if the index body cannot be parsed.
    pub fn trie(&self) -> eyre::Result<&'a ZeroTriePerfectHash<[u8]>> {
        Ok(self.parse_body()?.trie())
    }

    fn parse_body(&self) -> eyre::Result<ParsedSearchIndex<'a>> {
        let header = self.header()?;
        let terminal_count = usize::try_from(header.node_count).wrap_err_with(|| {
            format!(
                "Search index terminal count {} does not fit into usize",
                header.node_count
            )
        })?;

        if self.bytes.len() < SEARCH_INDEX_HEADER_LEN + SEARCH_INDEX_BODY_PREFIX_LEN {
            bail!(
                "Invalid search index body: expected at least {} bytes after header, got {}",
                SEARCH_INDEX_BODY_PREFIX_LEN,
                self.bytes.len().saturating_sub(SEARCH_INDEX_HEADER_LEN)
            );
        }

        let (mut cursor, segment_count, path_node_count, posting_row_id_count, trie_bytes) =
            self.parse_body_prefix()?;
        let segments = self.parse_segments(&mut cursor, segment_count)?;

        let layout = compute_search_index_layout(
            cursor,
            segment_count,
            path_node_count,
            terminal_count,
            posting_row_id_count,
        )?;
        let path_node_offset = layout.path_node;
        let terminal_offset = layout.terminal;
        let posting_range_offset = layout.posting_range;
        let posting_row_id_offset = layout.posting_row_id;
        let end = layout.end;
        if end != self.bytes.len() {
            bail!(
                "Corrupt search index: expected {} bytes total from encoded section counts, found {}",
                end,
                self.bytes.len()
            );
        }

        validate_path_nodes(self.bytes, path_node_offset, path_node_count, &segments)?;
        validate_terminal_rows(self.bytes, terminal_offset, terminal_count, path_node_count)?;
        validate_postings(
            self.bytes,
            posting_range_offset,
            posting_row_id_offset,
            segment_count,
            posting_row_id_count,
            terminal_count,
        )?;

        Ok(ParsedSearchIndex {
            bytes: self.bytes,
            trie_bytes,
            segments,
            path_node_offset,
            terminal_offset,
            terminal_count,
            posting_range_offset,
            posting_row_id_offset,
            posting_row_id_count,
        })
    }

    fn parse_body_prefix(&self) -> eyre::Result<(usize, usize, usize, usize, &'a [u8])> {
        let mut cursor = SEARCH_INDEX_HEADER_LEN;
        let segment_count = read_u32(self.bytes, &mut cursor) as usize;
        let path_node_count = read_u32(self.bytes, &mut cursor) as usize;
        let trie_len = read_u32(self.bytes, &mut cursor) as usize;
        let posting_row_id_count = read_u32(self.bytes, &mut cursor) as usize;

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

        Ok((
            cursor,
            segment_count,
            path_node_count,
            posting_row_id_count,
            trie_bytes,
        ))
    }

    fn parse_segments(
        &self,
        cursor: &mut usize,
        segment_count: usize,
    ) -> eyre::Result<Arc<[SearchIndexPathSegmentView<'a>]>> {
        let mut segments = Vec::with_capacity(segment_count);
        for segment_index in 0..segment_count {
            if self.bytes.len() < *cursor + 8 {
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

            let display =
                std::str::from_utf8(&self.bytes[*cursor..display_end]).wrap_err_with(|| {
                    format!("Invalid UTF-8 display segment payload at segment {segment_index}")
                })?;
            let normalized = std::str::from_utf8(&self.bytes[display_end..normalized_end])
                .wrap_err_with(|| {
                    format!("Invalid UTF-8 normalized segment payload at segment {segment_index}")
                })?;
            segments.push(SearchIndexPathSegmentView {
                display,
                normalized,
            });
            *cursor = normalized_end;
        }

        Ok(Arc::<[SearchIndexPathSegmentView<'a>]>::from(segments))
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

        let range_offset = self.posting_range_offset + segment_id * SEARCH_INDEX_POSTING_RANGE_LEN;
        let posting_start = read_u32_at(self.bytes, range_offset) as usize;
        let posting_len = read_u32_at(self.bytes, range_offset + 4) as usize;
        let posting_end = posting_start + posting_len;
        if posting_end > self.posting_row_id_count {
            bail!(
                "Corrupt search index: segment {} posting range {}..{} exceeds posting table length {}",
                segment_id,
                posting_start,
                posting_end,
                self.posting_row_id_count
            );
        }

        Ok(SearchIndexPostingIter {
            bytes: self.bytes,
            next_offset: self.posting_row_id_offset + posting_start * SEARCH_INDEX_POSTING_ROW_LEN,
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
    pub fn extend_rows<'a>(
        &mut self,
        rows: impl IntoIterator<Item = &'a SearchIndexPathRow>,
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
    end: usize,
}

fn serialize_search_index(
    header: SearchIndexHeader,
    rows: &[SearchIndexPathRow],
) -> eyre::Result<Vec<u8>> {
    let mut segment_ids_by_display = HashMap::<String, u32>::new();
    let mut segment_entries = Vec::<(String, String)>::new();
    let mut postings_by_segment = Vec::<Vec<u32>>::new();
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
            bail!("Cannot encode empty path into search index")
        }

        row_segment_ids.sort_unstable();
        row_segment_ids.dedup();
        for segment_id in row_segment_ids {
            postings_by_segment[segment_id as usize].push(row_index);
        }

        terminals.push((parent_node_index, row.has_deleted_entries));
    }

    let trie: ZeroTriePerfectHash<Vec<u8>> = normalized_trie_entries.into_iter().collect();
    let trie_bytes = trie.into_store();
    let segment_count: u32 = segment_entries.len().try_into().wrap_err_with(|| {
        format!(
            "Too many unique path segments to encode ({})",
            segment_entries.len()
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

    let mut bytes = Vec::with_capacity(SEARCH_INDEX_HEADER_LEN + rows.len() * 16);
    header.extend_vec(&mut bytes);
    bytes.extend_from_slice(&segment_count.to_le_bytes());
    bytes.extend_from_slice(&path_node_count.to_le_bytes());
    bytes.extend_from_slice(&trie_len.to_le_bytes());
    bytes.extend_from_slice(&posting_row_id_count.to_le_bytes());
    bytes.extend_from_slice(&trie_bytes);

    serialize_segment_entries(&mut bytes, &segment_entries)?;
    serialize_path_nodes(&mut bytes, &path_nodes);
    serialize_terminals(&mut bytes, &terminals);
    serialize_postings(&mut bytes, &postings_by_segment, posting_row_id_count)?;

    Ok(bytes)
}

fn compute_search_index_layout(
    cursor: usize,
    segment_count: usize,
    path_node_count: usize,
    terminal_count: usize,
    posting_row_id_count: usize,
) -> eyre::Result<SearchIndexLayout> {
    let path_node_bytes_len = path_node_count
        .checked_mul(SEARCH_INDEX_NODE_LEN)
        .ok_or_else(|| eyre::eyre!("Search index path-node table length overflow"))?;
    let terminal_bytes_len = terminal_count
        .checked_mul(SEARCH_INDEX_TERMINAL_LEN)
        .ok_or_else(|| eyre::eyre!("Search index terminal table length overflow"))?;
    let posting_range_bytes_len = segment_count
        .checked_mul(SEARCH_INDEX_POSTING_RANGE_LEN)
        .ok_or_else(|| eyre::eyre!("Search index posting-range table length overflow"))?;
    let posting_row_id_bytes_len = posting_row_id_count
        .checked_mul(SEARCH_INDEX_POSTING_ROW_LEN)
        .ok_or_else(|| eyre::eyre!("Search index posting row-id table length overflow"))?;

    let path_node_offset = cursor;
    let terminal_offset = path_node_offset + path_node_bytes_len;
    let posting_range_offset = terminal_offset + terminal_bytes_len;
    let posting_row_id_offset = posting_range_offset + posting_range_bytes_len;
    let end_offset = posting_row_id_offset + posting_row_id_bytes_len;

    Ok(SearchIndexLayout {
        path_node: path_node_offset,
        terminal: terminal_offset,
        posting_range: posting_range_offset,
        posting_row_id: posting_row_id_offset,
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

fn validate_postings(
    bytes: &[u8],
    posting_range_offset: usize,
    posting_row_id_offset: usize,
    segment_count: usize,
    posting_row_id_count: usize,
    terminal_count: usize,
) -> eyre::Result<()> {
    for segment_id in 0..segment_count {
        let range_offset = posting_range_offset + segment_id * SEARCH_INDEX_POSTING_RANGE_LEN;
        let posting_start = read_u32_at(bytes, range_offset) as usize;
        let posting_len = read_u32_at(bytes, range_offset + 4) as usize;
        let posting_end = posting_start + posting_len;
        if posting_end > posting_row_id_count {
            bail!(
                "Corrupt search index: segment {} posting range {}..{} exceeds posting table length {}",
                segment_id,
                posting_start,
                posting_end,
                posting_row_id_count
            );
        }

        for posting_index in posting_start..posting_end {
            let row_id_offset =
                posting_row_id_offset + posting_index * SEARCH_INDEX_POSTING_ROW_LEN;
            let row_id = read_u32_at(bytes, row_id_offset) as usize;
            if row_id >= terminal_count {
                bail!(
                    "Corrupt search index: segment {} posting references missing terminal row {}",
                    segment_id,
                    row_id
                );
            }
        }
    }

    Ok(())
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
    fn snapshot_small_index_bytes_v4() -> eyre::Result<()> {
        // This is synthetic test data serialized entirely in memory. The test
        // never reads or writes the path on disk.
        let rows = vec![SearchIndexPathRow {
            path: String::from(VIRTUAL_SNAPSHOT_TEST_PATH),
            has_deleted_entries: false,
        }];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 7, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;

        assert_eq!(
            bytes,
            vec![
                84, 77, 70, 84, 73, 68, 88, 0, 4, 0, 0, 0, 67, 7, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,
                0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 51, 0, 0, 0, 3, 0, 0, 0, 195, 95, 97, 113, 38,
                43, 95, 116, 101, 97, 109, 121, 95, 109, 102, 116, 95, 118, 105, 114, 116, 117, 97,
                108, 95, 115, 110, 97, 112, 115, 104, 111, 116, 95, 102, 105, 120, 116, 117, 114,
                101, 95, 95, 129, 46, 116, 120, 116, 130, 58, 128, 2, 0, 0, 0, 2, 0, 0, 0, 81, 58,
                113, 58, 38, 0, 0, 0, 38, 0, 0, 0, 95, 95, 84, 69, 65, 77, 89, 95, 77, 70, 84, 95,
                86, 73, 82, 84, 85, 65, 76, 95, 83, 78, 65, 80, 83, 72, 79, 84, 95, 70, 73, 88, 84,
                85, 82, 69, 95, 95, 95, 95, 116, 101, 97, 109, 121, 95, 109, 102, 116, 95, 118,
                105, 114, 116, 117, 97, 108, 95, 115, 110, 97, 112, 115, 104, 111, 116, 95, 102,
                105, 120, 116, 117, 114, 101, 95, 95, 5, 0, 0, 0, 5, 0, 0, 0, 97, 46, 116, 120,
                116, 97, 46, 116, 120, 116, 0, 0, 0, 0, 255, 255, 255, 255, 1, 0, 0, 0, 0, 0, 0, 0,
                2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0,
                0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]
        );

        let search_index_bytes = SearchIndexBytes::new(&bytes);
        assert_eq!(search_index_bytes.version()?, 4);

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

        assert_eq!(search_index_bytes.header()?.version, SEARCH_INDEX_VERSION);

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
