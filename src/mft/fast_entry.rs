//! Fast entry/attribute scanning helpers (filename-only focus).
//!
//! These utilities operate over raw entry byte slices (after fixups applied)
//! to extract `FILE_NAME` (0x30) attributes with minimal overhead.

use crate::mft::fast_fixup::detect_entry_size;
use crate::mft::mft_file::MftFile;
use crate::mft::mft_record_index::MftRecordIndex;
use rayon::prelude::*;
use tracing::debug_span;
use tracing::instrument;

pub const ATTR_TYPE_FILE_NAME: u32 = 0x30;
const ATTRIBUTE_TYPE_END: u32 = 0xFFFF_FFFF;
const RECORD_FLAGS_OFFSET: usize = 0x16;
const RECORD_FLAG_IN_USE: u16 = 0x0001;

#[derive(Clone, Copy, Debug)]
pub struct FileNameRef<'a> {
    pub entry_id: u32,
    pub parent_ref: u64, // raw 64-bit reference (contains sequence)
    pub namespace: u8,
    pub name_utf16: &'a [u16],
}

/// Collection of `FILE_NAME` attributes extracted from MFT data.
///
/// This structure provides organized access to all filename references found
/// in an MFT, with efficient lookup by entry ID.
#[derive(Clone, Debug)]
pub struct FileNameCollection<'a> {
    /// All `FILE_NAME` references found across all entries
    pub all_filenames: Vec<FileNameRef<'a>>,
    /// Index mapping where `per_entry[entry_id]` contains indices
    /// into `all_filenames` for all filenames belonging to that entry
    pub per_entry_indices: Vec<Vec<usize>>,
    /// Per-entry deleted state derived from MFT record flags (true when not in-use)
    pub per_entry_deleted: Vec<bool>,
}

impl<'a> FileNameCollection<'a> {
    /// Get all filename references for a specific entry ID.
    ///
    /// # Arguments
    ///
    /// * `entry_id` - The MFT entry ID to look up
    ///
    /// # Returns
    ///
    /// An iterator over all `FileNameRef` instances for the given entry,
    /// or an empty iterator if the entry ID is not found.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use teamy_mft::mft::{fast_entry, mft_file::MftFile};
    /// # fn demo() -> eyre::Result<()> {
    /// // Load an MFT file and collect all filename (x30) attributes
    /// let mft = MftFile::from_path(std::path::Path::new("C:\\path\\to\\cached.mft"))?;
    /// let collection = fast_entry::collect_filenames(&mft);
    /// for filename in collection.filenames_for_entry(5) {
    ///     println!("Entry 5 filename: {:?}", filename);
    /// }
    /// # Ok(()) }
    /// # let _ = demo();
    /// ```
    pub fn filenames_for_entry(&self, entry_id: u32) -> impl Iterator<Item = &FileNameRef<'a>> {
        self.per_entry_indices
            .get(entry_id as usize)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&idx| self.all_filenames.get(idx))
            })
            .into_iter()
            .flatten()
    }

    /// Get the total number of filename references collected.
    #[must_use]
    pub fn x30_count(&self) -> usize {
        self.all_filenames.len()
    }

    /// Get the number of entries that have filename attributes.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.per_entry_indices.len()
    }

    /// Returns true when the entry is marked deleted (not in-use) in MFT flags.
    #[must_use]
    pub fn is_entry_deleted(&self, entry_id: MftRecordIndex) -> bool {
        self.per_entry_deleted
            .get(entry_id.get())
            .copied()
            .unwrap_or(false)
    }
}

#[inline]
fn read_u16(bytes: &[u8], off: usize) -> Option<u16> {
    bytes
        .get(off..off + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
}
#[inline]
fn read_u32(bytes: &[u8], off: usize) -> Option<u32> {
    bytes
        .get(off..off + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
#[inline]
fn read_u64(bytes: &[u8], off: usize) -> Option<u64> {
    bytes
        .get(off..off + 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

#[inline]
fn is_entry_deleted(entry_bytes: &[u8]) -> bool {
    match read_u16(entry_bytes, RECORD_FLAGS_OFFSET) {
        Some(flags) => flags & RECORD_FLAG_IN_USE == 0,
        None => true,
    }
}

/// Parse the total entry size from the first entry slice (delegates to `fast_fixup` helper)
#[inline]
#[must_use]
pub fn parse_first_entry_size(first_entry: &[u8]) -> Option<u32> {
    detect_entry_size(first_entry)
}

/// Iterate all `FILE_NAME` attributes in an entry, invoking callback for each.
/// Returns number of filename attributes found.
#[cfg_attr(feature = "tracy", instrument(level = "debug", skip_all))]
// mftf[impl file-name-attributes.resident-x30]
pub fn for_each_filename<'a, F: FnMut(FileNameRef<'a>)>(
    entry_bytes: &'a [u8],
    entry_id: u32,
    mut f: F,
) -> usize {
    let first_attr_off = {
        #[cfg(feature = "tracy")]
        let _span = debug_span!("validate_entry_header").entered();
        if entry_bytes.len() < 0x18 || &entry_bytes[0..4] != b"FILE" {
            return 0;
        }
        let first_attr_off = match read_u16(entry_bytes, 0x14) {
            Some(v) => v as usize,
            None => return 0,
        };
        if first_attr_off == 0 || first_attr_off >= entry_bytes.len() {
            return 0;
        }
        first_attr_off
    };

    let mut offset = first_attr_off;
    let mut count = 0;
    while offset + 16 <= entry_bytes.len() {
        #[cfg(feature = "tracy")]
        let _span = debug_span!("scan_attribute_header").entered();
        // minimal attribute header length guard
        let Some(attr_type) = read_u32(entry_bytes, offset) else {
            break;
        };
        if attr_type == ATTRIBUTE_TYPE_END {
            break;
        }
        let attr_len = match read_u32(entry_bytes, offset + 4) {
            Some(v) => v as usize,
            None => break,
        };
        if attr_len == 0 || offset + attr_len > entry_bytes.len() {
            break;
        }
        let non_res_flag = entry_bytes.get(offset + 8).copied().unwrap_or(0);
        if attr_type == ATTR_TYPE_FILE_NAME && non_res_flag == 0 {
            let _span = debug_span!("parse_resident_file_name").entered();
            // Resident attribute header layout (offsets relative to attribute start)
            if offset + 24 > entry_bytes.len() {
                break;
            }
            let value_len = match read_u32(entry_bytes, offset + 16) {
                Some(v) => v as usize,
                None => break,
            };
            let value_off = match read_u16(entry_bytes, offset + 20) {
                Some(v) => v as usize,
                None => break,
            };
            let value_abs = offset + value_off;
            if value_abs + value_len > entry_bytes.len() || value_len < 0x42 { /* need base struct */
            } else {
                // FILE_NAME structure
                if let Some(parent_ref) = read_u64(entry_bytes, value_abs) {
                    let name_len = entry_bytes.get(value_abs + 0x40).copied().unwrap_or(0) as usize;
                    let namespace = entry_bytes.get(value_abs + 0x41).copied().unwrap_or(0);
                    let name_utf16_off = value_abs + 0x42;
                    let name_bytes_end = name_utf16_off + name_len * 2;
                    if name_bytes_end <= entry_bytes.len() {
                        #[allow(
                            clippy::cast_ptr_alignment,
                            reason = "NTFS data is expected to be properly aligned for u16"
                        )]
                        // SAFETY: `entry_bytes` contains validated NTFS FILE_NAME data with a proper size, and the bytes are expected to align for u16 reads.
                        let raw: &[u16] = unsafe {
                            std::slice::from_raw_parts(
                                entry_bytes[name_utf16_off..name_bytes_end]
                                    .as_ptr()
                                    .cast::<u16>(),
                                name_len,
                            )
                        };
                        f(FileNameRef {
                            entry_id,
                            parent_ref,
                            namespace,
                            name_utf16: raw,
                        });
                        count += 1;
                    }
                }
            }
        }
        offset += attr_len;
    }
    count
}

/// Parallel collection of all `FILE_NAME` attributes from MFT data.
///
/// This function processes MFT entries in parallel to extract all `FILE_NAME` attributes efficiently.
/// It's particularly useful for large MFT files where sequential processing would be too slow.
///
/// # Example
///
/// ```rust,no_run
/// use teamy_mft::mft::{fast_entry, mft_file::MftFile};
/// # fn demo() -> eyre::Result<()> {
/// let mft = MftFile::from_path(std::path::Path::new("C:\\path\\to\\cached.mft"))?;
/// let collection = fast_entry::collect_filenames(&mft);
/// // Access all filenames for entry ID 5
/// for filename in collection.filenames_for_entry(5) {
///     println!("Entry 5 has filename: {:?}", filename);
/// }
/// println!("Total filenames found: {}", collection.x30_count());
/// # Ok(()) }
/// # let _ = demo();
/// ```
/// # Panics
/// Panics if the MFT entry count exceeds `u32::MAX`.
#[instrument(level = "debug", skip_all)]
pub fn collect_filenames<'a>(mft: &'a MftFile) -> FileNameCollection<'a> {
    struct PerThreadData<'a> {
        file_names: Vec<FileNameRef<'a>>,
        pairs: Vec<(u32, usize)>,
        deleted_flags: Vec<(usize, bool)>,
    }

    let (full, entry_size, entry_count) = {
        let _span = debug_span!("prepare_collection_inputs").entered();
        let full: &'a [u8] = mft; // borrow the entire bytes buffer
        let entry_size = mft.record_size().get::<uom::si::information::byte>();
        let entry_count = mft.record_count();
        (full, entry_size, entry_count)
    };

    let per_thread = {
        let _span = debug_span!("parallel_collect_filenames").entered();
        (0..entry_count)
            .into_par_iter()
            .fold(
                || PerThreadData {
                    file_names: Vec::new(),
                    pairs: Vec::new(),
                    deleted_flags: Vec::new(),
                },
                |mut data, idx| {
                    #[cfg(feature = "tracy")]
                    let _span = debug_span!("scan_entry_for_filenames").entered();
                    let start = idx * entry_size;
                    let end = start + entry_size;
                    let record_bytes: &'a [u8] = &full[start..end];
                    data.deleted_flags
                        .push((idx, is_entry_deleted(record_bytes)));
                    for_each_filename(
                        record_bytes,
                        u32::try_from(idx).expect("idx should fit in u32"),
                        |fref| {
                            let local_index = data.file_names.len();
                            data.file_names.push(fref);
                            data.pairs.push((fref.entry_id, local_index));
                        },
                    );
                    data
                },
            )
            .collect::<Vec<_>>()
    };

    let per_entry_deleted = {
        let _span = debug_span!("build_deleted_flag_index").entered();
        let mut per_entry_deleted = vec![false; entry_count];
        for data in &per_thread {
            for &(entry_id, deleted) in &data.deleted_flags {
                per_entry_deleted[entry_id] = deleted;
            }
        }
        per_entry_deleted
    };

    let mut file_names = {
        let _span = debug_span!("flatten_thread_results").entered();
        let total = per_thread.iter().map(|data| data.file_names.len()).sum();
        let mut file_names = Vec::with_capacity(total);
        for data in &per_thread {
            file_names.extend_from_slice(&data.file_names);
        }
        file_names
    };

    let per_entry = {
        let _span = debug_span!("build_per_entry_index").entered();
        let mut per_entry: Vec<Vec<usize>> = vec![Vec::new(); entry_count];
        let mut base = 0usize;
        for data in per_thread {
            for (entry_id, local_idx) in data.pairs {
                let global_idx = base + local_idx;
                if let Some(vec) = per_entry.get_mut(entry_id as usize) {
                    vec.push(global_idx);
                }
            }
            base += data.file_names.len();
        }
        per_entry
    };

    {
        let _span = debug_span!("finalize_collection").entered();

        FileNameCollection {
            all_filenames: std::mem::take(&mut file_names),
            per_entry_indices: per_entry,
            per_entry_deleted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collect_filenames;
    use crate::mft::mft_file::MftFile;

    #[test]
    fn collect_filenames_tracks_deleted_flags_during_entry_scan() -> eyre::Result<()> {
        const ENTRY_SIZE: usize = 1024;
        let mut bytes = vec![0u8; ENTRY_SIZE * 2];
        bytes[0x1C..0x20].copy_from_slice(&(ENTRY_SIZE as u32).to_le_bytes());
        bytes[0..4].copy_from_slice(b"FILE");
        bytes[ENTRY_SIZE..ENTRY_SIZE + 4].copy_from_slice(b"FILE");
        bytes[0x16..0x18].copy_from_slice(&1u16.to_le_bytes());
        bytes[ENTRY_SIZE + 0x16..ENTRY_SIZE + 0x18].copy_from_slice(&0u16.to_le_bytes());

        let mft = MftFile::from_vec(bytes)?;
        let collection = collect_filenames(&mft);

        assert!(!collection.per_entry_deleted[0]);
        assert!(collection.per_entry_deleted[1]);
        Ok(())
    }
}
