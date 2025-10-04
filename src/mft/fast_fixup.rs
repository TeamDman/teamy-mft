//! Fast fixup application utilities for zero-copy / slice-based MFT processing.
//!
//! These functions operate directly on raw entry byte slices without constructing
//! higher level structs. They are intended for the high-performance pipeline
//! (mmap -> copy -> parallel fixups -> attribute scan).
//!
//! Design goals:
//! - Minimal branching in the hot path.
//! - No heap allocation per entry.
//! - Optional parallelism (feature `rayon`).
//!
//! Safety: All functions perform conservative bounds checks before touching
//! slice indices. No unsafe code is used here.

use std::time::Instant;
use thousands::Separable;
use tracing::debug;
use uom::si::f64::Information;
use uom::si::f64::Time;
use uom::si::frequency::hertz;
use uom::si::information::byte;
use uom::si::time::second;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FixupStats {
    pub applied: u64,
    pub already_applied: u64,
    pub invalid: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixupState {
    Applied,
    AlreadyApplied,
    Invalid,
}

impl FixupStats {
    #[inline]
    pub fn record(&mut self, state: FixupState) {
        match state {
            FixupState::Applied => self.applied += 1,
            FixupState::AlreadyApplied => self.already_applied += 1,
            FixupState::Invalid => self.invalid += 1,
        }
    }
}

const SECTOR: usize = 512; // NTFS logical sector (fixup stride)

/// Detect the entry size from the first entry bytes.
/// Expects a valid NTFS FILE record with a 1KB or 4KB typical size.
/// Returns None if slice too small.
#[inline]
pub fn detect_entry_size(entry0: &[u8]) -> Option<u32> {
    // total_entry_size at offset 0x1C (after used_entry_size at 0x18)
    if entry0.len() < 0x20 {
        return None;
    }
    let sz = u32::from_le_bytes(entry0[0x1C..0x20].try_into().ok()?);
    if sz == 0 {
        return None;
    }
    Some(sz)
}

#[inline]
fn read_update_sequence_array_fields(entry: &[u8]) -> Option<(usize, usize)> {
    if entry.len() < 8 {
        return None;
    }
    let usa_offset = u16::from_le_bytes([entry[4], entry[5]]) as usize;
    let usa_size = u16::from_le_bytes([entry[6], entry[7]]) as usize; // count of u16 elements (first is update sequence value)
    Some((usa_offset, usa_size))
}

/// Quick check if an entry still needs fixup application.
#[inline]
pub fn needs_fixup(entry: &[u8]) -> bool {
    let (usa_offset, usa_size) = match read_update_sequence_array_fields(entry) {
        Some(v) => v,
        None => return false,
    };
    if usa_size < 2 {
        return false;
    }
    let fixup_bytes_len = usa_size * 2; // each element u16
    if usa_offset + fixup_bytes_len > entry.len() {
        return false;
    }
    if entry.len() < SECTOR {
        return false;
    }
    let update_sequence = &entry[usa_offset..usa_offset + 2];
    &entry[SECTOR - 2..SECTOR] == update_sequence
}

/// Apply fixups in place for a single entry slice.
/// Returns the state of the operation.
#[inline]
pub fn apply_fixup_in_place(entry: &mut [u8]) -> FixupState {
    let (usa_offset, usa_size) = match read_update_sequence_array_fields(entry) {
        Some(v) => v,
        None => return FixupState::Invalid,
    };
    if usa_size < 2 {
        return FixupState::AlreadyApplied;
    }
    let total_fixup_bytes = usa_size * 2;
    if usa_offset + total_fixup_bytes > entry.len() {
        return FixupState::Invalid;
    }

    let update_sequence = {
        let start = usa_offset;
        let end = start + 2;
        entry[start..end].to_vec()
    }; // own copy to avoid borrow conflicts
    let original_bytes = {
        let start = usa_offset + 2;
        let end = usa_offset + total_fixup_bytes;
        entry[start..end].to_vec()
    };
    let sectors = usa_size - 1; // first element reserved for update sequence value

    let mut any_applied = false;
    for i in 0..sectors {
        let sector_end = (i + 1) * SECTOR;
        if sector_end > entry.len() || sector_end < 2 {
            return if any_applied {
                FixupState::Applied
            } else {
                FixupState::Invalid
            };
        }
        let tail_start = sector_end - 2;
        // Avoid simultaneous immutable/mutable borrow; split slice.
        let (head, tail_and_rest) = entry.split_at_mut(tail_start);
        let tail = &mut tail_and_rest[..2];
        // head unused; keeps borrows disjoint.
        let _ = head;

        if tail == &update_sequence[..] {
            let fix_slice = &original_bytes[i * 2..i * 2 + 2];
            tail.copy_from_slice(fix_slice);
            any_applied = true;
        } else {
            let original = &original_bytes[i * 2..i * 2 + 2];
            if tail != original {
                return FixupState::Invalid;
            }
        }
    }

    if any_applied {
        FixupState::Applied
    } else {
        FixupState::AlreadyApplied
    }
}

/// Apply fixups to all entries in the buffer using parallelism when the `rayon` feature is enabled.
/// Also logs basic telemetry: detected entry size, entry count, elapsed time and throughput, and outcome stats.
pub fn apply_fixups_parallel(buf: &mut [u8], entry_size: usize) -> FixupStats {
    if entry_size == 0 || !buf.len().is_multiple_of(entry_size) {
        debug!(
            "Invalid/unaligned entry size: entry_size={} buf_len={}",
            entry_size,
            buf.len()
        );
        return FixupStats::default();
    }

    let entry_count = buf.len() / entry_size;
    debug!(
        "Detected entry size: {} bytes, total entries: {}",
        entry_size.separate_with_commas(),
        entry_count.separate_with_commas()
    );

    let start = Instant::now();

    use rayon::prelude::*;
    let stats = buf
        .par_chunks_mut(entry_size)
        .map(|entry| {
            if entry.len() < entry_size {
                return FixupState::Invalid;
            }
            apply_fixup_in_place(entry)
        })
        .fold(FixupStats::default, |mut acc, state| {
            acc.record(state);
            acc
        })
        .reduce(FixupStats::default, |a, b| FixupStats {
            applied: a.applied + b.applied,
            already_applied: a.already_applied + b.already_applied,
            invalid: a.invalid + b.invalid,
        });

    let elapsed = Time::new::<second>(start.elapsed().as_secs_f64());
    let total_size = Information::new::<byte>(buf.len() as f64);
    let rate = total_size / elapsed;
    debug!(
        "Took {elapsed} to process {count} records ({rate}/s) - fixup stats: applied={applied} already-applied={already_applied} invalid={invalid}",
        elapsed = elapsed.get_human(),
        count = entry_count.separate_with_commas(),
        rate = rate.get::<hertz>().trunc().separate_with_commas(),
        applied = stats.applied.separate_with_commas(),
        already_applied = stats.already_applied.separate_with_commas(),
        invalid = stats.invalid.separate_with_commas()
    );

    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_entry_size_basic() {
        // Minimal fake header with total_entry_size = 1024 at 0x1C
        let mut entry = vec![0u8; 0x20];
        entry[0..4].copy_from_slice(b"FILE");
        entry[0x1C..0x20].copy_from_slice(&1024u32.to_le_bytes());
        assert_eq!(detect_entry_size(&entry), Some(1024));
    }
}
