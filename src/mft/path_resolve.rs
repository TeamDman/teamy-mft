//! Basic sequential path resolution over collected `FileNameRef` entries.
//! This is a first-pass simple implementation (non-parallel) to be optimized later.

use crate::mft::fast_entry::FileNameCollection;
use std::borrow::Cow;
use std::path::PathBuf;

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
pub struct MftEntryPathCollection(pub Vec<Vec<PathBuf>>);
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
    pub fn paths_for(&self, entry_id: usize) -> &[PathBuf] {
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
///
/// # Errors
///
/// Currently this function always returns `Ok`, but the fallible signature allows future
/// extensions that might fail during decoding or validation.
#[allow(clippy::too_many_lines, reason = "complex path resolution logic")]
pub fn resolve_paths_all_parallel(
    file_names: &FileNameCollection<'_>,
) -> eyre::Result<MftEntryPathCollection> {
    use rayon::prelude::*;
    let entry_count = file_names.entry_count();

    // Build raw selections with namespace precedence (same logic as sequential version) then decode.
    let mut raw: Vec<Vec<(usize, u8, &'_ [u16])>> = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        raw.push(Vec::new());
    }
    for (entry_id, list) in raw.iter_mut().enumerate() {
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
    let mut per_entry: Vec<Vec<BestName>> = Vec::with_capacity(entry_count);
    for raw_list in &raw {
        let mut v: Vec<BestName> = Vec::with_capacity(raw_list.len());
        for (parent, _ns, name_units) in raw_list {
            v.push(BestName {
                parent: *parent,
                name: decode_name(name_units).into_owned(),
            });
        }
        per_entry.push(v);
    }

    // Compute depth (minimum parent depth + 1) so parents always processed before children.
    let mut depth: Vec<i32> = vec![-1; entry_count];
    let mut mark: Vec<Mark> = vec![Mark::Unvis; entry_count];
    for i in 0..entry_count {
        if depth[i] == -1 {
            dfs(i, &per_entry, &mut depth, &mut mark);
        }
    }
    #[allow(clippy::cast_sign_loss, reason = "depth is non-negative")]
    let max_depth = depth.iter().copied().max().unwrap_or(0) as usize;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_depth + 1];
    for (i, d) in depth.iter().enumerate() {
        #[allow(clippy::cast_sign_loss, reason = "depth is non-negative")]
        layers[*d as usize].push(i);
    }

    // Results storage
    let mut results: Vec<Vec<PathBuf>> = vec![Vec::new(); entry_count];
    if entry_count > 5 {
        results[5].push(PathBuf::new());
    }

    // Process each layer: build outputs in parallel (read-only borrow of earlier results) then write.
    for layer_ids in &layers {
        let layer_outputs: Vec<(usize, Vec<PathBuf>)> = layer_ids
            .par_iter()
            .map(|&entry_id| {
                if !results[entry_id].is_empty() {
                    return (entry_id, Vec::new());
                }
                let mut acc: Vec<PathBuf> = Vec::new();
                for bn in &per_entry[entry_id] {
                    if bn.parent == entry_id {
                        continue;
                    }
                    let parent_paths = &results[bn.parent];
                    if parent_paths.is_empty() {
                        continue;
                    }
                    for parent_path in parent_paths {
                        let mut p = parent_path.clone();
                        p.push(&bn.name);
                        acc.push(p);
                    }
                }
                if acc.len() > 1 {
                    acc.sort();
                    acc.dedup();
                }
                (entry_id, acc)
            })
            .collect();
        // Write phase
        for (id, acc) in layer_outputs {
            if !acc.is_empty() {
                results[id] = acc;
            }
        }
    }

    Ok(MftEntryPathCollection(results))
}
