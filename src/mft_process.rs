use crate::mft::fast_entry;
use crate::mft::mft_file::MftFile;
use crate::mft::path_resolve;
use crate::mft::path_resolve::MftEntryPathCollection;
use eyre::Result;
use std::path::Path;
use std::time::Instant;
use thousands::Separable;
use tracing::debug;
use tracing::info;
use uom::si::f64::Information;
use uom::si::f64::InformationRate;
use uom::si::f64::Ratio;
use uom::si::f64::Time;
use uom::si::frequency::hertz;
use uom::si::information::byte;
use uom::si::ratio::ratio;
use uom::si::time::second;

/// Overall high-level processing of an MFT file: mmap -> copy -> fixups -> extract names -> resolve paths.
pub fn process_mft_file(
    drive_letter: &str,
    mft_file_path: &Path,
) -> Result<MftEntryPathCollection> {
    info!(
        drive_letter = &drive_letter,
        "Processing MFT file: {}",
        mft_file_path.display()
    );

    let start = std::time::Instant::now();

    let mft_file = MftFile::from_path(mft_file_path)?;

    // collect filename attributes (parallel)
    let scan_start = Instant::now();
    let file_names =
        fast_entry::par_collect_filenames(&mft_file, mft_file.entry_size().get::<byte>() as usize);
    let scan_elapsed = Time::new::<second>(scan_start.elapsed().as_secs_f64());
    let scan_rate = InformationRate::from(mft_file.size() / scan_elapsed);
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({}) entries_with_names={}",
        scan_elapsed.get_human(),
        scan_rate.get_human(),
        file_names.entry_count().separate_with_commas()
    );

    // resolve paths (multi-parent default)
    debug!(
        drive_letter = &drive_letter,
        "Resolving (multi-parent) paths for {} filename attributes",
        file_names.x30_count().separate_with_commas()
    );
    let path_resolve_start = Instant::now();
    let multi =        path_resolve::resolve_paths_all_parallel(&file_names)?;
    let path_resolve_elapsed = Time::new::<second>(path_resolve_start.elapsed().as_secs_f64());
    let total_paths = multi.total_paths();
    let resolved_entries = multi.0.iter().filter(|v| !v.is_empty()).count();
    let resolve_rate = InformationRate::from(
        Information::new::<byte>(resolved_entries as f64 * 256.0) / path_resolve_elapsed,
    );
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({}) entries_resolved={} total_paths={}",
        path_resolve_elapsed.get_human(),
        resolve_rate.get_human(),
        resolved_entries.separate_with_commas(),
        total_paths.separate_with_commas()
    );

    debug!(
        drive_letter = &drive_letter,
        "MFT {}: size={} entries={} entry_size={} names={} resolved={} timings(scan/resolve)={}/{}",
        mft_file_path.display(),
        mft_file.size().get_human(),
        mft_file.entry_count().separate_with_commas(),
        mft_file.entry_size().get_human(),
        file_names.x30_count().separate_with_commas(),
        resolved_entries.separate_with_commas(),
        scan_elapsed.get_human(),
        path_resolve_elapsed.get_human()
    );

    let elapsed = Time::new::<second>(start.elapsed().as_secs_f64());
    // aggregate performance statistics
    let total_data_rate = InformationRate::from(mft_file.size() / elapsed); // overall throughput
    let entries_rate = Ratio::new::<ratio>(mft_file.entry_count() as f64) / elapsed;
    debug!(
        drive_letter = &drive_letter,
        "Total processing time for {} with {} entries: {} (size={} rate={} entries/s={})",
        mft_file_path.display(),
        mft_file.entry_count().separate_with_commas(),
        elapsed.get_human(),
        mft_file.size().get_human(),
        total_data_rate.get_human(),
        entries_rate.get::<hertz>().trunc().separate_with_commas()
    );
    Ok(multi)
}
