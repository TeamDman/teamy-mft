use crate::mft::fast_entry;
use crate::mft::mft_file::MftFile;
use crate::mft::path_resolve;
use crate::mft::path_resolve::MftEntryPathCollection;
use eyre::Result;
use humansize::BINARY;
use std::path::Path;
use std::time::Instant;
use teamy_uom_extensions::HumanInformationExt;
use teamy_uom_extensions::HumanInformationRateExt;
use teamy_uom_extensions::HumanTimeExt;
use teamy_uom_extensions::InformationOverExt;
use thousands::Separable;
use tracing::debug;
use uom::si::f64::Ratio;
use uom::si::f64::Time;
use uom::si::frequency::hertz;
use uom::si::information::byte;
use uom::si::ratio::ratio;
use uom::si::time::second;

/// Overall high-level processing of an MFT file: mmap -> copy -> fixups -> extract names -> resolve paths.
///
/// # Errors
///
/// Returns an error if reading or parsing the cached MFT data fails.
pub fn process_mft_file(
    drive_letter: &str,
    mft_file_path: &Path,
) -> Result<MftEntryPathCollection> {
    debug!(
        drive_letter = &drive_letter,
        "Processing MFT file: {}",
        mft_file_path.display()
    );

    let start = std::time::Instant::now();

    let mft_file = MftFile::from_path(mft_file_path)?;

    // collect filename attributes (parallel)
    let scan_start = Instant::now();
    let file_names = fast_entry::collect_filenames(&mft_file);
    let scan_elapsed = Time::new::<second>(scan_start.elapsed().as_secs_f64());
    let scan_rate = mft_file.size().over(scan_elapsed);
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({}) entries_with_names={}",
        scan_elapsed.format_human(),
        scan_rate.format_human(BINARY),
        file_names.entry_count().separate_with_commas()
    );

    // resolve paths (multi-parent default)
    debug!(
        drive_letter = &drive_letter,
        "Resolving (multi-parent) paths for {} filename attributes",
        file_names.x30_count().separate_with_commas()
    );
    let path_resolve_start = Instant::now();
    let multi = path_resolve::resolve_paths_all_parallel(&file_names)?;
    let path_resolve_elapsed = Time::new::<second>(path_resolve_start.elapsed().as_secs_f64());
    let total_paths = multi.total_paths();
    let resolved_entries = multi.0.iter().filter(|v| !v.is_empty()).count();
    #[allow(clippy::cast_precision_loss, reason = "counts may exceed 2^52 but we only use them for rate reporting")]
    let resolved_entries_f64 = resolved_entries as f64;
    let resolved_size = uom::si::f64::Information::new::<byte>(resolved_entries_f64 * 256.0);
    let resolve_rate = resolved_size.over(path_resolve_elapsed);
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({}) entries_resolved={} total_paths={}",
        path_resolve_elapsed.format_human(),
        resolve_rate.format_human(BINARY),
        resolved_entries.separate_with_commas(),
        total_paths.separate_with_commas()
    );

    debug!(
        drive_letter = &drive_letter,
        "MFT {}: size={} entries={} entry_size={} names={} resolved={} timings(scan/resolve)={}/{}",
        mft_file_path.display(),
        mft_file.size().format_human(BINARY),
        mft_file.record_count().separate_with_commas(),
        mft_file.record_size().format_human(BINARY),
        file_names.x30_count().separate_with_commas(),
        resolved_entries.separate_with_commas(),
        scan_elapsed.format_human(),
        path_resolve_elapsed.format_human()
    );

    let elapsed = Time::new::<second>(start.elapsed().as_secs_f64());
    // aggregate performance statistics
    #[allow(clippy::cast_precision_loss, reason = "double precision rate math is best-effort for large MFT sizes")]
    let total_size = uom::si::f64::Information::new::<byte>(mft_file.size().get::<byte>() as f64);
    let total_data_rate = total_size.over(elapsed); // overall throughput
    #[allow(clippy::cast_precision_loss, reason = "double precision entry rate math is best-effort for high entry counts")]
    let entries_rate = Ratio::new::<ratio>(mft_file.record_count() as f64) / elapsed;
    debug!(
        drive_letter = &drive_letter,
        "Total processing time for {} with {} entries: {} (size={} rate={} entries/s={})",
        mft_file_path.display(),
        mft_file.record_count().separate_with_commas(),
        elapsed.format_human(),
        mft_file.size().format_human(BINARY),
        total_data_rate.format_human(BINARY),
        entries_rate.get::<hertz>().trunc().separate_with_commas()
    );
    Ok(multi)
}
