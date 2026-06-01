use crate::machine::config::PublishedCheckpoint;
use crate::machine::config::PublishedDrivePaths;
use crate::machine::config::current_unix_ms;
use crate::machine::config::load_checkpoint;
use crate::machine::config::save_checkpoint;
use crate::machine::ipc::MachineError;
use crate::machine::usn::JournalCursor;
use crate::machine::usn::UsnEvent;
use crate::machine::usn::VolumeUsnJournalHandle;
use crate::mft::fast_entry;
use crate::mft::mft_file::MftFile;
use crate::mft::mft_record_reference::MftRecordReference;
use crate::mft::mft_sequence_number::MftSequenceNumber;
use crate::query::QueryFilter;
use crate::query::QueryIgnoreRules;
use crate::query::QueryPlan;
use crate::query::QueryResultRow;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use crate::search_index::search_index_bytes::SearchIndexBytes;
use crate::search_index::search_index_bytes::SearchIndexBytesMut;
use eyre::Context;
use eyre::ContextCompat;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use tracing::instrument;
use tracing::trace;
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveNodeLink {
    parent_frn: u64,
    name: String,
    is_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveNode {
    is_directory: bool,
    links: Vec<LiveNodeLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveDriveGraph {
    drive_letter: char,
    root_frn: u64,
    nodes: FxHashMap<u64, LiveNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectedPath {
    path: String,
    is_live: bool,
}

#[derive(Debug)]
pub struct LiveDriveState {
    pub drive_letter: char,
    sync_dir: PathBuf,
    pub paths: PublishedDrivePaths,
    volume_serial_number: Option<u32>,
    snapshot_usn: u64,
    published_last_usn: u64,
    current_next_usn: u64,
    journal_id: u64,
    base_source_mft_len_bytes: u64,
    base_rows: Vec<SearchIndexPathRow>,
    current_graph: LiveDriveGraph,
    current_rows_cache: Option<Vec<SearchIndexPathRow>>,
    current_index_bytes_cache: Option<Vec<u8>>,
    overlay_rows_cache: Option<Vec<SearchIndexPathRow>>,
    overlay_index_bytes_cache: Option<Vec<u8>>,
    published_dirty: bool,
    query_cache_dirty: bool,
}

impl LiveDriveState {
    /// # Errors
    ///
    /// Returns an error if the published snapshot/checkpoint cannot be loaded or the
    /// USN journal continuity check fails.
    #[instrument(level = "debug", skip_all, fields(drive = %paths.drive_letter))]
    pub fn load(sync_dir: &Path, paths: PublishedDrivePaths) -> eyre::Result<Self> {
        Self::load_with_cancel(sync_dir, paths, None)
    }

    /// # Errors
    ///
    /// Returns an error if the published snapshot/checkpoint cannot be loaded, the
    /// USN journal continuity check fails, or cancellation is requested.
    #[instrument(level = "debug", skip_all, fields(drive = %paths.drive_letter))]
    pub fn load_with_cancel(
        sync_dir: &Path,
        paths: PublishedDrivePaths,
        cancel: Option<&AtomicBool>,
    ) -> eyre::Result<Self> {
        let checkpoint = load_checkpoint(&paths.checkpoint_path)?.wrap_err_with(|| {
            format!(
                "Missing checkpoint for drive {} at {}",
                paths.drive_letter,
                paths.checkpoint_path.display()
            )
        })?;

        let snapshot_usn = checkpoint
            .snapshot_usn
            .or(checkpoint.last_usn)
            .wrap_err_with(|| {
                format!(
                    "Checkpoint for drive {} does not include a snapshot or published USN",
                    paths.drive_letter
                )
            })?;
        let journal_id = checkpoint.journal_id.wrap_err_with(|| {
            format!(
                "Checkpoint for drive {} does not include a journal id",
                paths.drive_letter
            )
        })?;

        let journal = VolumeUsnJournalHandle::open(paths.drive_letter)?;
        let cursor = journal.query_cursor()?;
        validate_journal_continuity(paths.drive_letter, &checkpoint, cursor)?;

        let _span = info_span!(
            "load_live_drive_base",
            drive = %paths.drive_letter,
            snapshot_usn,
            published_last_usn = checkpoint.last_usn.unwrap_or(snapshot_usn),
            journal_id
        )
        .entered();

        let mft_file =
            MftFile::from_path_with_cancel(&paths.mft_path, cancel).wrap_err_with(|| {
                format!(
                    "Failed loading base MFT snapshot for drive {} from {}",
                    paths.drive_letter,
                    paths.mft_path.display()
                )
            })?;
        let base_graph =
            LiveDriveGraph::from_mft_with_cancel(paths.drive_letter, &mft_file, cancel)?;
        let base_rows = load_rows_from_index_path(&paths.base_index_path)?;
        let base_source_mft_len_bytes = mft_file.size().get::<uom::si::information::byte>() as u64;

        let mut state = Self {
            drive_letter: paths.drive_letter,
            sync_dir: sync_dir.to_path_buf(),
            paths,
            volume_serial_number: checkpoint.volume_serial_number,
            snapshot_usn,
            published_last_usn: checkpoint.last_usn.unwrap_or(snapshot_usn),
            current_next_usn: snapshot_usn,
            journal_id,
            base_source_mft_len_bytes,
            base_rows,
            current_graph: base_graph,
            current_rows_cache: None,
            current_index_bytes_cache: None,
            overlay_rows_cache: None,
            overlay_index_bytes_cache: None,
            published_dirty: false,
            query_cache_dirty: true,
        };

        state.refresh_from_journal_with_cancel(&journal, cancel)?;
        Ok(state)
    }

    #[must_use]
    pub fn published_dirty(&self) -> bool {
        self.published_dirty
    }

    #[must_use]
    pub fn published_last_usn(&self) -> u64 {
        self.published_last_usn
    }

    #[must_use]
    pub fn snapshot_usn(&self) -> u64 {
        self.snapshot_usn
    }

    #[must_use]
    pub fn current_next_usn(&self) -> u64 {
        self.current_next_usn
    }

    /// # Errors
    ///
    /// Returns an error if reading additional journal records fails.
    #[instrument(level = "debug", skip_all, fields(drive = %self.drive_letter, current_next_usn = self.current_next_usn))]
    pub fn refresh(&mut self) -> eyre::Result<()> {
        self.refresh_with_cancel(None)
    }

    /// # Errors
    ///
    /// Returns an error if reading additional journal records fails or cancellation is requested.
    #[instrument(level = "debug", skip_all, fields(drive = %self.drive_letter, current_next_usn = self.current_next_usn))]
    pub fn refresh_with_cancel(&mut self, cancel: Option<&AtomicBool>) -> eyre::Result<()> {
        let journal = VolumeUsnJournalHandle::open(self.drive_letter)?;
        let cursor = journal.query_cursor()?;
        validate_active_cursor(
            self.drive_letter,
            self.snapshot_usn,
            self.journal_id,
            self.current_next_usn,
            cursor,
        )?;
        self.refresh_from_journal_with_cancel(&journal, cancel)
    }

    /// # Errors
    ///
    /// Returns an error if the current in-memory index cannot be built or queried.
    #[instrument(level = "debug", skip_all, fields(drive = %self.drive_letter, query = ?request.query))]
    pub fn query(&mut self, request: &QueryPlan) -> Result<Vec<QueryResultRow>, MachineError> {
        self.query_with_cancel(request, None)
    }

    /// # Errors
    ///
    /// Returns an error if the current in-memory index cannot be built or queried.
    #[instrument(level = "debug", skip_all, fields(drive = %self.drive_letter, query = ?request.query))]
    pub fn query_with_cancel(
        &mut self,
        request: &QueryPlan,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<QueryResultRow>, MachineError> {
        let ignore_rules =
            QueryIgnoreRules::discover_for_drive_letters(&[self.drive_letter], &self.sync_dir)
                .map_err(|error| MachineError::degraded(error.to_string()))?;
        let filter = QueryFilter::new(request, Some(ignore_rules))
            .map_err(|error| MachineError::request_invalid(error.to_string()))?;

        let limit = request.limit.get();
        let mut rows = Vec::with_capacity(limit.unwrap_or_default());
        self.current_graph.visit_projected_rows(|projected| {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                return false;
            }
            let row = QueryResultRow {
                path: projected.path.into(),
                has_deleted_entries: !projected.is_live,
                is_ignored: false,
            };
            if !request.query.matches(row.path.as_str()) {
                return true;
            }
            if let Some(row) = filter.classify_and_match(row) {
                rows.push(row);
                if limit.is_some_and(|limit| rows.len() >= limit) {
                    return false;
                }
            }
            true
        });
        Ok(rows)
    }

    /// # Errors
    ///
    /// Returns an error if the overlay index or checkpoint cannot be written.
    #[instrument(level = "info", skip_all, fields(drive = %self.drive_letter, published_last_usn = self.published_last_usn, current_next_usn = self.current_next_usn))]
    pub fn flush_published(&mut self) -> eyre::Result<()> {
        self.ensure_full_query_cache()?;
        let overlay_rows = self
            .overlay_rows_cache
            .as_deref()
            .wrap_err("Missing overlay row cache")?;
        let overlay_index = self
            .overlay_index_bytes_cache
            .as_deref()
            .wrap_err("Missing overlay index cache")?;

        write_search_index_bytes(&self.paths.overlay_index_path, overlay_index)?;
        let checkpoint = PublishedCheckpoint {
            drive_letter: self.drive_letter,
            volume_serial_number: self.volume_serial_number,
            journal_id: Some(self.journal_id),
            snapshot_usn: Some(self.snapshot_usn),
            last_usn: Some(self.current_next_usn),
            published_at_unix_ms: current_unix_ms(),
            overlay_row_count: overlay_rows.len() as u64,
            base_index_version: SEARCH_INDEX_VERSION,
        };
        save_checkpoint(&self.paths.checkpoint_path, &checkpoint)?;
        self.published_last_usn = self.current_next_usn;
        self.published_dirty = false;
        info!(
            drive = %self.drive_letter,
            overlay_row_count = overlay_rows.len(),
            published_last_usn = self.published_last_usn,
            "Flushed overlay index and checkpoint"
        );
        Ok(())
    }

    fn refresh_from_journal_with_cancel(
        &mut self,
        journal: &VolumeUsnJournalHandle,
        cancel: Option<&AtomicBool>,
    ) -> eyre::Result<()> {
        let batch = journal.read_available_since_with_cancel(
            self.current_next_usn,
            self.journal_id,
            cancel,
        )?;
        if batch.next_usn == self.current_next_usn {
            trace!(
                drive = %self.drive_letter,
                current_next_usn = self.current_next_usn,
                "No new journal events to apply"
            );
            self.published_dirty = self.current_next_usn != self.published_last_usn;
            return Ok(());
        }

        let mut applied_events = 0usize;
        for event in batch.events {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                eyre::bail!(
                    "Cancelled applying USN journal events for drive {}",
                    self.drive_letter
                );
            }
            if !event.affects_topology() {
                continue;
            }
            applied_events += 1;
            self.current_graph.apply_event(&event);
        }

        self.current_next_usn = batch.next_usn;
        self.published_dirty = self.current_next_usn != self.published_last_usn;
        if applied_events > 0 {
            self.query_cache_dirty = true;
            debug!(
                drive = %self.drive_letter,
                applied_events,
                current_next_usn = self.current_next_usn,
                "Applied live journal events to drive graph"
            );
        }
        Ok(())
    }

    fn ensure_current_query_cache(&mut self) -> eyre::Result<()> {
        if !self.query_cache_dirty
            && self.current_rows_cache.is_some()
            && self.current_index_bytes_cache.is_some()
        {
            return Ok(());
        }

        let _span = info_span!(
            "rebuild_live_drive_current_query_cache",
            drive = %self.drive_letter,
            published_dirty = self.published_dirty
        )
        .entered();
        let current_rows = self.current_graph.project_rows();
        let current_index_bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(
                self.drive_letter,
                self.base_source_mft_len_bytes,
                current_rows.len() as u64,
            ),
            &current_rows,
        )?
        .into_inner()?;

        debug!(
            drive = %self.drive_letter,
            current_row_count = current_rows.len(),
            "Rebuilt in-memory query cache"
        );

        self.current_rows_cache = Some(current_rows);
        self.current_index_bytes_cache = Some(current_index_bytes);
        self.overlay_rows_cache = None;
        self.overlay_index_bytes_cache = None;
        self.query_cache_dirty = false;
        Ok(())
    }

    fn ensure_full_query_cache(&mut self) -> eyre::Result<()> {
        self.ensure_current_query_cache()?;
        if self.overlay_rows_cache.is_some() && self.overlay_index_bytes_cache.is_some() {
            return Ok(());
        }

        let _span = info_span!(
            "rebuild_live_drive_overlay_cache",
            drive = %self.drive_letter,
            published_dirty = self.published_dirty
        )
        .entered();
        let current_rows = self
            .current_rows_cache
            .as_deref()
            .wrap_err("Missing current row cache")?;
        let overlay_rows = diff_overlay_rows(&self.base_rows, current_rows);
        let overlay_index_bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(
                self.drive_letter,
                self.base_source_mft_len_bytes,
                overlay_rows.len() as u64,
            ),
            &overlay_rows,
        )?
        .into_inner()?;

        debug!(
            drive = %self.drive_letter,
            overlay_row_count = overlay_rows.len(),
            "Rebuilt in-memory overlay cache"
        );

        self.overlay_rows_cache = Some(overlay_rows);
        self.overlay_index_bytes_cache = Some(overlay_index_bytes);
        Ok(())
    }
}

impl LiveDriveGraph {
    fn from_mft_with_cancel(
        drive_letter: char,
        mft_file: &MftFile,
        cancel: Option<&AtomicBool>,
    ) -> eyre::Result<Self> {
        let file_names = fast_entry::collect_filenames(mft_file);
        let records = mft_file.iter_records().collect::<Vec<_>>();
        let frns = records
            .iter()
            .map(|record| {
                MftRecordReference::from_parts(
                    record.get_record_number(),
                    MftSequenceNumber::new(record.get_sequence_number()),
                )
                .to_raw()
            })
            .collect::<Vec<_>>();
        let root_frn = frns
            .get(5)
            .copied()
            .wrap_err("MFT snapshot missing root directory record")?;

        let mut nodes = FxHashMap::<u64, LiveNode>::default();
        for (entry_id, record) in records.iter().enumerate() {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
                eyre::bail!("Cancelled building live graph for drive {drive_letter}");
            }
            let frn = frns[entry_id];
            let is_deleted = record.is_deleted();
            let node = nodes.entry(frn).or_insert_with(|| LiveNode {
                is_directory: record.flags().is_directory(),
                links: Vec::new(),
            });
            node.is_directory = record.flags().is_directory();

            let mut best_by_parent = FxHashMap::<u64, (u8, String)>::default();
            let entry_id =
                u32::try_from(entry_id).expect("MFT entry ids should fit in a u32 row space");
            for name_ref in file_names.filenames_for_entry(entry_id) {
                best_by_parent
                    .entry(name_ref.parent_ref)
                    .and_modify(|(namespace, current_name)| {
                        if namespace_rank(name_ref.namespace) < namespace_rank(*namespace) {
                            *namespace = name_ref.namespace;
                            *current_name = decode_utf16_lossy(name_ref.name_utf16);
                        }
                    })
                    .or_insert_with(|| {
                        (name_ref.namespace, decode_utf16_lossy(name_ref.name_utf16))
                    });
            }

            let mut links = best_by_parent
                .into_iter()
                .map(|(parent_frn, (_, name))| LiveNodeLink {
                    parent_frn,
                    name,
                    is_deleted,
                })
                .collect::<Vec<_>>();
            links.sort_by(|left, right| {
                left.parent_frn
                    .cmp(&right.parent_frn)
                    .then_with(|| left.name.cmp(&right.name))
            });
            node.links = links;
        }

        Ok(Self {
            drive_letter,
            root_frn,
            nodes,
        })
    }

    fn apply_event(&mut self, event: &UsnEvent) {
        let node = self.nodes.entry(event.frn).or_insert_with(|| LiveNode {
            is_directory: event.is_directory(),
            links: Vec::new(),
        });
        node.is_directory = event.is_directory();

        let is_live_reason = event.reason
            & (crate::machine::usn::USN_REASON_FILE_CREATE
                | crate::machine::usn::USN_REASON_RENAME_NEW_NAME
                | crate::machine::usn::USN_REASON_HARD_LINK_CHANGE)
            != 0;
        let is_deleted_reason = event.reason
            & (crate::machine::usn::USN_REASON_FILE_DELETE
                | crate::machine::usn::USN_REASON_RENAME_OLD_NAME)
            != 0;

        if is_live_reason {
            if let Some(link) = node
                .links
                .iter_mut()
                .find(|link| link.parent_frn == event.parent_frn && link.name == event.name)
            {
                link.is_deleted = false;
            } else {
                node.links.push(LiveNodeLink {
                    parent_frn: event.parent_frn,
                    name: event.name.clone(),
                    is_deleted: false,
                });
            }
        }

        if is_deleted_reason {
            if let Some(link) = node
                .links
                .iter_mut()
                .find(|link| link.parent_frn == event.parent_frn && link.name == event.name)
            {
                link.is_deleted = true;
            } else {
                node.links.push(LiveNodeLink {
                    parent_frn: event.parent_frn,
                    name: event.name.clone(),
                    is_deleted: true,
                });
            }
        }
    }

    fn project_rows(&self) -> Vec<SearchIndexPathRow> {
        self.projected_rows()
            .into_iter()
            .map(|projected| SearchIndexPathRow {
                path: projected.path,
                has_deleted_entries: !projected.is_live,
            })
            .collect()
    }

    fn projected_rows(&self) -> Vec<ProjectedPath> {
        let mut memo = FxHashMap::<u64, Vec<ProjectedPath>>::default();
        let mut visiting = FxHashSet::<u64>::default();
        let mut path_states = BTreeMap::<String, bool>::new();
        for frn in self.nodes.keys().copied() {
            for projected in self.projected_paths_for(frn, &mut memo, &mut visiting) {
                path_states
                    .entry(projected.path)
                    .and_modify(|is_live| *is_live |= projected.is_live)
                    .or_insert(projected.is_live);
            }
        }

        path_states
            .into_iter()
            .map(|(path, is_live)| ProjectedPath { path, is_live })
            .collect()
    }

    fn visit_projected_rows(&self, mut visit: impl FnMut(ProjectedPath) -> bool) {
        let mut memo = FxHashMap::<u64, Vec<ProjectedPath>>::default();
        let mut visiting = FxHashSet::<u64>::default();
        for frn in self.nodes.keys().copied() {
            for projected in self.projected_paths_for(frn, &mut memo, &mut visiting) {
                if !visit(projected) {
                    return;
                }
            }
        }
    }

    fn projected_paths_for(
        &self,
        frn: u64,
        memo: &mut FxHashMap<u64, Vec<ProjectedPath>>,
        visiting: &mut FxHashSet<u64>,
    ) -> Vec<ProjectedPath> {
        if let Some(cached) = memo.get(&frn) {
            return cached.clone();
        }
        if !visiting.insert(frn) {
            warn!(drive = %self.drive_letter, frn, "Detected cycle while projecting drive graph");
            return Vec::new();
        }

        let root_path = format!("{}:\\", self.drive_letter);
        let projected = if frn == self.root_frn {
            vec![ProjectedPath {
                path: root_path,
                is_live: true,
            }]
        } else {
            let Some(node) = self.nodes.get(&frn) else {
                visiting.remove(&frn);
                return Vec::new();
            };
            let mut by_path = FxHashMap::<String, bool>::default();
            for link in &node.links {
                let parent_paths = self.projected_paths_for(link.parent_frn, memo, visiting);
                for parent in parent_paths {
                    let path = join_windows_path(&parent.path, &link.name);
                    let is_live = parent.is_live && !link.is_deleted;
                    by_path
                        .entry(path)
                        .and_modify(|existing| *existing |= is_live)
                        .or_insert(is_live);
                }
            }
            let mut projected = by_path
                .into_iter()
                .map(|(path, is_live)| ProjectedPath { path, is_live })
                .collect::<Vec<_>>();
            projected.sort_by(|left, right| left.path.cmp(&right.path));
            projected
        };

        visiting.remove(&frn);
        memo.insert(frn, projected.clone());
        projected
    }
}

fn validate_journal_continuity(
    drive_letter: char,
    checkpoint: &PublishedCheckpoint,
    cursor: JournalCursor,
) -> eyre::Result<()> {
    let snapshot_usn = checkpoint
        .snapshot_usn
        .or(checkpoint.last_usn)
        .wrap_err_with(|| {
            format!("Checkpoint for drive {drive_letter} is missing a replay cursor")
        })?;
    let journal_id = checkpoint
        .journal_id
        .wrap_err_with(|| format!("Checkpoint for drive {drive_letter} is missing a journal id"))?;
    validate_active_cursor(
        drive_letter,
        snapshot_usn,
        journal_id,
        checkpoint.last_usn.unwrap_or(snapshot_usn),
        cursor,
    )
}

fn validate_active_cursor(
    drive_letter: char,
    snapshot_usn: u64,
    journal_id: u64,
    current_usn: u64,
    cursor: JournalCursor,
) -> eyre::Result<()> {
    if cursor.journal_id != journal_id {
        eyre::bail!(
            "USN journal for drive {} was reset (expected id {}, found {})",
            drive_letter,
            journal_id,
            cursor.journal_id
        );
    }
    if snapshot_usn < cursor.lowest_valid_usn {
        eyre::bail!(
            "USN replay gap for drive {}: snapshot_usn={} fell below lowest_valid_usn={}",
            drive_letter,
            snapshot_usn,
            cursor.lowest_valid_usn
        );
    }
    if current_usn > cursor.next_usn {
        eyre::bail!(
            "Checkpoint for drive {} is ahead of the current journal head ({} > {})",
            drive_letter,
            current_usn,
            cursor.next_usn
        );
    }
    Ok(())
}

#[allow(
    clippy::redundant_closure_for_method_calls,
    reason = "The explicit closure keeps the Result mapping readable at this boundary"
)]
fn load_rows_from_index_path(path: &Path) -> eyre::Result<Vec<SearchIndexPathRow>> {
    let bytes = std::fs::read(path)
        .wrap_err_with(|| format!("Failed reading search index rows from {}", path.display()))?;
    SearchIndexBytes::new(&bytes)
        .row_views()?
        .map(|row| row.map(|view| view.to_owned()))
        .collect()
}

fn write_search_index_bytes(path: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let temp_path = path.with_extension("mft_overlay_search_index.tmp");
    std::fs::write(&temp_path, bytes).wrap_err_with(|| {
        format!(
            "Failed writing temporary overlay search index {}",
            temp_path.display()
        )
    })?;
    std::fs::rename(&temp_path, path).wrap_err_with(|| {
        format!(
            "Failed atomically replacing overlay search index {}",
            path.display()
        )
    })?;
    Ok(())
}

fn diff_overlay_rows(
    base_rows: &[SearchIndexPathRow],
    current_rows: &[SearchIndexPathRow],
) -> Vec<SearchIndexPathRow> {
    let base_by_path = base_rows
        .iter()
        .map(|row| (row.path.clone(), row.has_deleted_entries))
        .collect::<FxHashMap<_, _>>();
    let current_by_path = current_rows
        .iter()
        .map(|row| (row.path.clone(), row.has_deleted_entries))
        .collect::<FxHashMap<_, _>>();

    let mut overlay = current_by_path
        .iter()
        .filter_map(
            |(path, &has_deleted_entries)| match base_by_path.get(path) {
                Some(base_deleted) if *base_deleted == has_deleted_entries => None,
                _ => Some(SearchIndexPathRow {
                    path: path.clone(),
                    has_deleted_entries,
                }),
            },
        )
        .collect::<Vec<_>>();

    overlay.extend(
        base_by_path
            .keys()
            .filter(|path| !current_by_path.contains_key(*path))
            .map(|path| SearchIndexPathRow {
                path: path.clone(),
                has_deleted_entries: true,
            }),
    );
    overlay.sort_by(|left, right| left.path.cmp(&right.path));
    overlay
}

fn join_windows_path(parent: &str, child: &str) -> String {
    if parent.ends_with('\\') {
        format!("{parent}{child}")
    } else {
        format!("{parent}\\{child}")
    }
}

fn decode_utf16_lossy(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

fn namespace_rank(namespace: u8) -> u8 {
    match namespace {
        1 => 0,
        3 => 1,
        0 => 2,
        2 => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::LiveDriveGraph;
    use super::LiveDriveState;
    use super::LiveNode;
    use super::LiveNodeLink;
    use super::current_unix_ms;
    use super::diff_overlay_rows;
    use super::join_windows_path;
    use super::validate_active_cursor;
    use crate::machine::config::published_drive_paths;
    use crate::machine::daemon::sync_machine_cache;
    use crate::machine::usn::JournalCursor;
    use crate::machine::usn::UsnEvent;
    use crate::query::QueryPlan;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::sync::IfExistsOutputBehaviour;
    use eyre::ContextCompat;
    use rustc_hash::FxHashMap;
    use std::time::Duration;

    fn base_graph() -> LiveDriveGraph {
        let mut nodes = FxHashMap::default();
        nodes.insert(
            5,
            LiveNode {
                is_directory: true,
                links: Vec::new(),
            },
        );
        nodes.insert(
            10,
            LiveNode {
                is_directory: false,
                links: vec![LiveNodeLink {
                    parent_frn: 5,
                    name: String::from("alpha.txt"),
                    is_deleted: false,
                }],
            },
        );
        LiveDriveGraph {
            drive_letter: 'C',
            root_frn: 5,
            nodes,
        }
    }

    #[test]
    fn join_windows_path_preserves_root_separator() {
        assert_eq!(join_windows_path(r"C:\", "alpha.txt"), r"C:\alpha.txt");
        assert_eq!(
            join_windows_path(r"C:\tmp", "alpha.txt"),
            r"C:\tmp\alpha.txt"
        );
    }

    #[test]
    fn rename_events_project_old_path_deleted_and_new_path_live() {
        let mut graph = base_graph();
        graph.apply_event(&UsnEvent {
            frn: 10,
            parent_frn: 5,
            usn: 11,
            reason: crate::machine::usn::USN_REASON_RENAME_OLD_NAME,
            file_attributes: 0,
            name: String::from("alpha.txt"),
        });
        graph.apply_event(&UsnEvent {
            frn: 10,
            parent_frn: 5,
            usn: 12,
            reason: crate::machine::usn::USN_REASON_RENAME_NEW_NAME,
            file_attributes: 0,
            name: String::from("beta.txt"),
        });

        let rows = graph.project_rows();
        assert!(
            rows.iter()
                .any(|row| row.path == r"C:\beta.txt" && !row.has_deleted_entries)
        );
        assert!(
            rows.iter()
                .any(|row| row.path == r"C:\alpha.txt" && row.has_deleted_entries)
        );
    }

    #[test]
    fn overlay_diff_marks_removed_base_paths_deleted() {
        let base_rows = vec![SearchIndexPathRow {
            path: String::from(r"C:\alpha.txt"),
            has_deleted_entries: false,
        }];
        let current_rows = vec![SearchIndexPathRow {
            path: String::from(r"C:\beta.txt"),
            has_deleted_entries: false,
        }];
        let overlay = diff_overlay_rows(&base_rows, &current_rows);
        assert_eq!(overlay.len(), 2);
        assert!(
            overlay
                .iter()
                .any(|row| row.path == r"C:\beta.txt" && !row.has_deleted_entries)
        );
        assert!(
            overlay
                .iter()
                .any(|row| row.path == r"C:\alpha.txt" && row.has_deleted_entries)
        );
    }

    #[test]
    fn live_query_filters_projected_paths_by_query_text() -> eyre::Result<()> {
        let cache_dir = tempfile::tempdir()?;
        let mut graph = base_graph();
        graph.nodes.insert(
            11,
            LiveNode {
                is_directory: false,
                links: vec![LiveNodeLink {
                    parent_frn: 5,
                    name: String::from("music.flac"),
                    is_deleted: false,
                }],
            },
        );
        let mut state = LiveDriveState {
            drive_letter: 'C',
            sync_dir: cache_dir.path().to_path_buf(),
            paths: published_drive_paths(cache_dir.path(), 'C'),
            volume_serial_number: None,
            snapshot_usn: 0,
            published_last_usn: 0,
            current_next_usn: 0,
            journal_id: 1,
            base_source_mft_len_bytes: 0,
            base_rows: Vec::new(),
            current_graph: graph,
            current_rows_cache: None,
            current_index_bytes_cache: None,
            overlay_rows_cache: None,
            overlay_index_bytes_cache: None,
            published_dirty: false,
            query_cache_dirty: true,
        };

        let rows = state
            .query(&QueryPlan::new("music"))
            .map_err(|error| eyre::eyre!(error.message))?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path.as_str(), r"C:\music.flac");
        Ok(())
    }

    #[test]
    fn active_cursor_validation_rejects_gaps() {
        let error = validate_active_cursor(
            'C',
            100,
            5,
            110,
            JournalCursor {
                journal_id: 5,
                first_usn: 0,
                next_usn: 120,
                lowest_valid_usn: 101,
                max_usn: 999,
            },
        )
        .expect_err("gap should be rejected");
        assert!(error.to_string().contains("replay gap"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires NTFS journal access, elevation, and a full-drive MFT sync"]
    fn live_refresh_observes_new_file_after_base_sync() -> eyre::Result<()> {
        let scope_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let drive_letter = scope_dir
            .path()
            .to_string_lossy()
            .chars()
            .next()
            .wrap_err("failed extracting drive letter from temp dir")?;
        let needle = format!("__teamy_mft_live_refresh_{}__", current_unix_ms());
        let created_path = scope_dir.path().join(format!("{needle}.txt"));

        sync_machine_cache(
            cache_dir.path(),
            &[drive_letter],
            IfExistsOutputBehaviour::Overwrite,
        )?;

        let mut state = LiveDriveState::load(
            cache_dir.path(),
            published_drive_paths(cache_dir.path(), drive_letter),
        )?;
        let base_request = QueryPlan {
            r#in: Some(scope_dir.path().to_string_lossy().into_owned()),
            include_deleted: true,
            show_ignored: true,
            ..QueryPlan::new(needle.clone())
        };
        assert!(
            state
                .query(&base_request)
                .map_err(|error| eyre::eyre!(error.message.clone()))?
                .is_empty()
        );

        std::fs::write(&created_path, b"hello from live refresh")?;
        std::thread::sleep(Duration::from_millis(250));
        state.refresh()?;
        let rows = state
            .query(&base_request)
            .map_err(|error| eyre::eyre!(error.message.clone()))?;
        assert!(
            rows.iter().any(|row| row
                .path
                .as_str()
                .eq_ignore_ascii_case(&created_path.to_string_lossy())),
            "expected query to include {}, rows were {:?}",
            created_path.display(),
            rows
        );

        Ok(())
    }
}
