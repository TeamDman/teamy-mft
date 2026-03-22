//! Basic sequential path resolution over collected `FileNameRef` entries.
//! This is a first-pass simple implementation (non-parallel) to be optimized later.

use crate::mft::fast_entry::FileNameCollection;
use crate::mft::mft_record_index::MftRecordIndex;
use std::borrow::Cow;
use std::path::Path;
use std::path::PathBuf;
use tracing::debug_span;
use tracing::instrument;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub path: PathBuf,
    pub root_prefix: String,
    pub components: Vec<String>,
    pub component_deleted: Vec<bool>,
}

impl ResolvedPath {
    #[must_use]
    pub fn has_deleted_entries(&self) -> bool {
        self.component_deleted.iter().any(|is_deleted| *is_deleted)
    }

    #[must_use]
    fn deleted_segment_count(&self) -> usize {
        self.component_deleted
            .iter()
            .filter(|is_deleted| **is_deleted)
            .count()
    }
}

/// Decode UTF-16 little endian slice to String (lossy ASCII fast-path optional later).
fn decode_name(units: &[u16]) -> Cow<'_, str> {
    use std::char::decode_utf16;
    // ASCII fast path: if all code units are < 0x80 build directly
    if units.iter().all(|&u| u < 0x80) {
        let mut s = String::with_capacity(units.len());
        #[allow(clippy::cast_possible_truncation, reason = "checked u < 0x80")]
        for &u in units {
            s.push(u as u8 as char);
        }
        return Cow::Owned(s);
    }
    let iter = decode_utf16(units.iter().copied());
    let mut s = String::with_capacity(units.len());
    for r in iter {
        s.push(r.unwrap_or('\u{FFFD}'));
    }
    Cow::Owned(s)
}

/// A mapping from MFT entry ID to zero/one/many resolved paths.
/// Because an entry can have multiple x30 attributes, one entry may have more than one full path associated with it.
#[derive(Debug, Default, Clone)]
pub struct MftEntryPathCollection(pub Vec<Vec<ResolvedPath>>);
impl MftEntryPathCollection {
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.0.len()
    }
    #[must_use]
    pub fn total_paths(&self) -> usize {
        self.0.iter().map(std::vec::Vec::len).sum()
    }
    #[must_use]
    pub fn paths_for(&self, entry_id: usize) -> &[ResolvedPath] {
        self.0.get(entry_id).map_or(&[], |v| &**v)
    }
}

#[inline]
fn ns_rank(ns: u8) -> u8 {
    match ns {
        1 => 0,
        3 => 1,
        0 => 2,
        2 => 3,
        _ => 4,
    }
} // Win32 > Win32AndDos > POSIX > DOS

#[derive(Clone)]
struct BestName {
    parent: usize,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Unvis,
    Temp,
    Done,
}

fn dfs(i: usize, per_entry: &Vec<Vec<BestName>>, depth: &mut [i32], mark: &mut [Mark]) -> i32 {
    if mark[i] == Mark::Done {
        return depth[i];
    }
    if mark[i] == Mark::Temp {
        return 0;
    } // cycle/self-root
    mark[i] = Mark::Temp;
    let mut best = 0;
    for bn in &per_entry[i] {
        if bn.parent == i {
            continue;
        }
        let pd = dfs(bn.parent, per_entry, depth, mark);
        if pd + 1 > best {
            best = pd + 1;
        }
    }
    depth[i] = best;
    mark[i] = Mark::Done;
    best
}

/// Resolve all paths including multiple hardlink parents.
/// For each distinct parent of an entry, keep only the highest-precedence namespace.
/// Returns zero/one/many paths per entry (index aligned with entry id).
/// Paths are rooted at `root_prefix` (e.g., `C:\`) so absolute paths are produced directly.
///
/// # Errors
///
/// Currently this function always returns `Ok`, but the fallible signature allows future
/// extensions that might fail during decoding or validation.
#[allow(clippy::too_many_lines, reason = "complex path resolution logic")]
#[instrument(level = "debug", skip(file_names))]
pub fn resolve_paths_all_parallel(
    file_names: &FileNameCollection<'_>,
    root_prefix: &Path,
) -> eyre::Result<MftEntryPathCollection> {
    use rayon::prelude::*;
    let entry_count = {
        let _span = debug_span!("get_entry_count").entered();
        file_names.entry_count()
    };

    // Build raw selections with namespace precedence (same logic as sequential version) then decode.
    let raw = {
        let _span = debug_span!("build_raw_parent_name_selection").entered();
        let mut raw: Vec<Vec<(usize, u8, &'_ [u16])>> = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            raw.push(Vec::new());
        }
        for (entry_id, list) in raw.iter_mut().enumerate() {
            #[cfg(feature = "tracy")]
            let _span = debug_span!("collect_entry_raw_names").entered();
            #[allow(clippy::cast_possible_truncation, reason = "entry_id fits in u32")]
            for fref in file_names.filenames_for_entry(entry_id as u32) {
                let parent = (fref.parent_ref & 0xFFFF_FFFF_FFFF) as usize;
                if parent >= entry_count {
                    continue;
                }
                if let Some((_, ns, name_units)) = list.iter_mut().find(|(p, _, _)| *p == parent) {
                    if ns_rank(fref.namespace) < ns_rank(*ns) {
                        *ns = fref.namespace;
                        *name_units = fref.name_utf16;
                    }
                } else {
                    list.push((parent, fref.namespace, fref.name_utf16));
                }
            }
        }
        raw
    };

    let per_entry = {
        let _span = debug_span!("decode_raw_names_to_best_names").entered();
        let mut per_entry: Vec<Vec<BestName>> = Vec::with_capacity(entry_count);
        for raw_list in &raw {
            #[cfg(feature = "tracy")]
            let _span = debug_span!("decode_entry_names").entered();
            let mut v: Vec<BestName> = Vec::with_capacity(raw_list.len());
            for (parent, _ns, name_units) in raw_list {
                v.push(BestName {
                    parent: *parent,
                    name: decode_name(name_units).into_owned(),
                });
            }
            per_entry.push(v);
        }
        per_entry
    };

    // Compute depth (minimum parent depth + 1) so parents always processed before children.
    let layers = {
        let _span = debug_span!("build_depth_layers").entered();
        let mut depth: Vec<i32> = vec![-1; entry_count];
        let mut mark: Vec<Mark> = vec![Mark::Unvis; entry_count];
        for i in 0..entry_count {
            if depth[i] == -1 {
                #[cfg(feature = "tracy")]
                let _span = debug_span!("dfs_depth").entered();
                dfs(i, &per_entry, &mut depth, &mut mark);
            }
        }

        #[allow(clippy::cast_sign_loss, reason = "depth is non-negative")]
        let max_depth = depth.iter().copied().max().unwrap_or(0) as usize;
        let mut layers: Vec<Vec<MftRecordIndex>> = vec![Vec::new(); max_depth + 1];
        for (i, d) in depth.iter().enumerate() {
            #[allow(clippy::cast_sign_loss, reason = "depth is non-negative")]
            layers[*d as usize].push(MftRecordIndex::new(i));
        }
        layers
    };

    // Results storage
    let mut results = {
        let _span = debug_span!("initialize_results_storage").entered();
        let root_prefix_display = root_prefix.to_string_lossy().into_owned();
        let mut results: Vec<Vec<ResolvedPath>> = vec![Vec::new(); entry_count];
        if entry_count > 5 {
            let _span = debug_span!("seed_root_entry").entered();
            results[5].push(ResolvedPath {
                path: root_prefix.to_path_buf(),
                root_prefix: root_prefix_display,
                components: Vec::new(),
                component_deleted: Vec::new(),
            });
        }
        results
    };

    // Process each layer: build outputs in parallel (read-only borrow of earlier results) then write.
    {
        let _span = debug_span!("resolve_paths_by_layers").entered();
        for layer_ids in &layers {
            let _span = debug_span!("resolve_single_layer").entered();
            let layer_outputs: Vec<(MftRecordIndex, Vec<ResolvedPath>)> = layer_ids
                .par_iter()
                .map(|&entry_id| {
                    let _span = debug_span!("resolve_single_entry").entered();
                    if !results[entry_id.get()].is_empty() {
                        return (entry_id, Vec::new());
                    }
                    let mut acc: Vec<ResolvedPath> = Vec::new();
                    {
                        #[cfg(feature = "tracy")]
                        let _span = debug_span!("expand_parent_paths").entered();
                        for bn in &per_entry[entry_id.get()] {
                            if bn.parent == entry_id.get() {
                                continue;
                            }
                            let parent_paths = &results[bn.parent];
                            if parent_paths.is_empty() {
                                continue;
                            }
                            for parent_path in parent_paths {
                                let mut p = parent_path.path.clone();
                                p.push(&bn.name);
                                let mut components = parent_path.components.clone();
                                components.push(bn.name.clone());
                                let mut component_deleted = parent_path.component_deleted.clone();
                                component_deleted.push(file_names.is_entry_deleted(entry_id));
                                acc.push(ResolvedPath {
                                    path: p,
                                    root_prefix: parent_path.root_prefix.clone(),
                                    components,
                                    component_deleted,
                                });
                            }
                        }
                    }
                    if acc.len() > 1 {
                        let _span = debug_span!("dedup_entry_paths").entered();
                        let mut dedup = rustc_hash::FxHashMap::<PathBuf, ResolvedPath>::default();
                        for candidate in acc {
                            dedup
                                .entry(candidate.path.clone())
                                .and_modify(|existing| {
                                    if candidate.deleted_segment_count()
                                        < existing.deleted_segment_count()
                                    {
                                        *existing = candidate.clone();
                                    }
                                })
                                .or_insert(candidate);
                        }
                        let mut deduped = dedup.into_values().collect::<Vec<_>>();
                        deduped.sort_by(|left, right| left.path.cmp(&right.path));
                        acc = deduped;
                    }
                    (entry_id, acc)
                })
                .collect();
            {
                let _span = debug_span!("write_layer_results").entered();
                for (id, acc) in layer_outputs {
                    if !acc.is_empty() {
                        results[id.get()] = acc;
                    }
                }
            }
        }
    }

    Ok(MftEntryPathCollection(results))
}
