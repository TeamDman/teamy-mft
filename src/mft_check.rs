use crate::drive_letter_pattern::DriveLetterPattern;
use crate::sync_dir::try_get_sync_dir;
use eyre::Result;
use eyre::WrapErr;
use memmap2::Mmap;
use mft::fast_entry;
use mft::fast_fixup;
use mft::path_resolve;
use uom::si::ratio::ratio;
use std::path::Path;
use std::path::PathBuf;
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
use uom::si::time::second;

/// Statistics and timing information for processing a single MFT file.
pub struct MftProcessStats {
    pub path: PathBuf,
    pub mft_file_size: Information,
    pub entry_size: Information,
    pub entry_count: usize,
    pub fixups_applied: u64,
    pub fixups_already: u64,
    pub fixups_invalid: u64,
    pub filename_attrs: usize,
    pub resolved_paths: usize,
    pub dur_fixups: Time,
    pub dur_scan: Time,
    pub dur_resolve: Time,
    pub sample_paths: Vec<PathBuf>,
}

pub fn check_drives(drive_letter_pattern: DriveLetterPattern, parallel: bool) -> eyre::Result<()> {
    // Get MFT files from sync dir
    let sync_dir = try_get_sync_dir()?;
    let drive_letters: Vec<char> = drive_letter_pattern.into_drive_letters()?;
    debug!(
        "Pattern {:?} gave drive letters: {:?}",
        drive_letter_pattern, drive_letters
    );
    let mft_files: Vec<(char, PathBuf)> = drive_letters
        .into_iter()
        .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
        .filter(|(_, p)| p.is_file())
        .collect();
    debug!(
        "Checking MFT files: {:#?}",
        mft_files.iter().map(|(_, p)| p).collect::<Vec<_>>()
    );

    if parallel {
        use std::sync::Arc;
        let handles: Vec<_> = mft_files
            .into_iter()
            .map(|(drive_letter, mft_file_path)| {
                let path = Arc::new(mft_file_path);
                let d = drive_letter.to_string();
                std::thread::spawn(move || process_mft_file(d, &path, 10, true))
            })
            .collect();
        let mut first_err: Option<eyre::Report> = None;
        for h in handles {
            match h.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => { if first_err.is_none() { first_err = Some(e); } }
                Err(panic) => {
                    let msg = if let Some(s) = panic.downcast_ref::<&str>() { *s } else if let Some(s) = panic.downcast_ref::<String>() { s.as_str() } else { "unknown panic" };
                    return Err(eyre::eyre!("Thread panicked: {msg}"));
                }
            }
        }
        if let Some(e) = first_err { return Err(e); }
    } else {
        for (drive_letter, mft_file_path) in mft_files {
            process_mft_file(drive_letter.to_string(), &mft_file_path, 10, false)?;
        }
    }
    Ok(())
}

/// Overall high-level processing of an MFT file: mmap -> copy -> fixups -> extract names -> resolve paths.
pub fn process_mft_file(
    drive_letter: String,
    mft_file_path: &Path,
    sample_limit: usize,
    parallel: bool,
) -> Result<MftProcessStats> {
    info!(
        drive_letter = &drive_letter,
        "Processing MFT file: {}",
        mft_file_path.display()
    );

    let start = std::time::Instant::now();

    // open file
    let file = std::fs::File::open(mft_file_path)
        .with_context(|| format!("Failed to open {}", mft_file_path.display()))?;
    debug!(
        drive_letter = &drive_letter,
        "Opened MFT file: {}",
        mft_file_path.display()
    );

    // file size
    let file_size_bytes = file
        .metadata()
        .with_context(|| format!("Failed to get metadata for {}", mft_file_path.display()))?
        .len() as usize;
    let mft_file_size = Information::new::<byte>(file_size_bytes as f64);
    if file_size_bytes < 1024 {
        eyre::bail!("MFT file too small: {}", mft_file_path.display());
    }

    // mmap
    debug!(
        drive_letter = &drive_letter,
        "Memory-mapping {}",
        mft_file_size.get_human()
    );
    let mmap_start = Instant::now();
    let mmap = unsafe { Mmap::map(&file) }
        .with_context(|| format!("Failed to memory-map {}", mft_file_path.display()))?;
    let bytes: &[u8] = &mmap;
    let mmap_elapsed = Time::new::<second>(mmap_start.elapsed().as_secs_f64());
    let mmap_rate = InformationRate::from(mft_file_size / mmap_elapsed);
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({})",
        mmap_elapsed.get_human(),
        mmap_rate.get_human()
    );
    assert_eq!(file_size_bytes, bytes.len());

    // detect entry size
    debug!(
        drive_letter = &drive_letter,
        "Detecting entry size for {}",
        mft_file_path.display()
    );
    let entry_size_bytes = fast_fixup::detect_entry_size(&bytes[0..1024]).unwrap_or(1024) as usize;
    let entry_size = Information::new::<byte>(entry_size_bytes as f64);
    if entry_size_bytes == 0 || bytes.len() % entry_size_bytes != 0 {
        eyre::bail!("Unaligned entry size for {}", mft_file_path.display());
    }
    let entry_count = bytes.len() / entry_size_bytes;
    debug!(
        drive_letter = &drive_letter,
        "Detected entry size: {} bytes, total entries: {}", entry_size_bytes, entry_count
    );

    // copy bytes (allows in-place fixup)
    debug!(
        drive_letter = &drive_letter,
        "Copying {} to owned Vec",
        mft_file_size.get_human()
    );
    let start_copy = Instant::now();
    let mut owned = Vec::with_capacity(bytes.len());
    owned.extend_from_slice(bytes);
    let copy_elapsed = Time::new::<second>(start_copy.elapsed().as_secs_f64());
    let copy_rate = InformationRate::from(mft_file_size / copy_elapsed);
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({})",
        copy_elapsed.get_human(),
        copy_rate.get_human()
    );

    // apply fixups (parallel)
    debug!(
        drive_letter = &drive_letter,
        "Applying fixups to {} entries",
        entry_count.separate_with_commas()
    );
    let fixup_start = Instant::now();
    let stats = fast_fixup::apply_fixups_parallel(&mut owned, entry_size_bytes);
    let fixup_elapsed = Time::new::<second>(fixup_start.elapsed().as_secs_f64());
    let fixup_rate = mft_file_size / fixup_elapsed;
    debug!(
        drive_letter = &drive_letter,
        "Took {} ({}/s) applied/already/invalid={}/{}/{}",
        fixup_elapsed.get_human(),
        fixup_rate.get::<hertz>().trunc().separate_with_commas(),
        stats.applied.separate_with_commas(),
        stats.already_applied.separate_with_commas(),
        stats.invalid.separate_with_commas()
    );

    // collect filename attributes (parallel)
    debug!(
        drive_letter = &drive_letter,
        "Collecting filename attributes from {} entries",
        entry_count.separate_with_commas()
    );
    let scan_start = Instant::now();
    let file_names = fast_entry::par_collect_filenames(&owned, entry_size_bytes);
    let scan_elapsed = Time::new::<second>(scan_start.elapsed().as_secs_f64());
    let scan_rate = InformationRate::from(mft_file_size / scan_elapsed);
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
    let multi = if parallel { path_resolve::resolve_paths_all_parallel(&file_names)? } else { path_resolve::resolve_paths_all(&file_names)? };
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

    // sample (flatten multi paths)
    let sample_paths: Vec<PathBuf> = multi
        .0
        .iter()
        .flat_map(|v| v.iter())
        .take(sample_limit)
        .cloned()
        .collect();

    let rtn = MftProcessStats {
        path: mft_file_path.to_path_buf(),
        mft_file_size,
        entry_size,
        entry_count,
        fixups_applied: stats.applied,
        fixups_already: stats.already_applied,
        fixups_invalid: stats.invalid,
        filename_attrs: file_names.x30_count(),
        resolved_paths: resolved_entries, // now counts entries with at least one path
        dur_fixups: fixup_elapsed,
        dur_scan: scan_elapsed,
        dur_resolve: path_resolve_elapsed,
        sample_paths,
    };
    info!(
        drive_letter = &drive_letter,
        "MFT {}: size={} entries={} entry_size={} fixups(applied/already/invalid)={}/{}/{} names={} resolved={} timings(fix/scan/resolve)={}/{}/{}",
        rtn.path.display(),
        mft_file_size.get_human(),
        rtn.entry_count.separate_with_commas(),
        rtn.entry_size.get_human(),
        rtn.fixups_applied.separate_with_commas(),
        rtn.fixups_already.separate_with_commas(),
        rtn.fixups_invalid.separate_with_commas(),
        rtn.filename_attrs.separate_with_commas(),
        rtn.resolved_paths.separate_with_commas(),
        rtn.dur_fixups.get_human(),
        rtn.dur_scan.get_human(),
        rtn.dur_resolve.get_human()
    );
    for p in &rtn.sample_paths {
        info!("PATH: {}:\\{}", drive_letter, p.display());
    }

    let elapsed = Time::new::<second>(start.elapsed().as_secs_f64());
    // aggregate performance statistics
    let total_data_rate = InformationRate::from(mft_file_size / elapsed); // overall throughput
    let entries_rate = Ratio::new::<ratio>(rtn.entry_count as f64) / elapsed;
    info!(
        drive_letter = &drive_letter,
        "Total processing time for {} with {} entries: {} (size={} rate={} entries/s={})",
        mft_file_path.display(),
        rtn.entry_count.separate_with_commas(),
        elapsed.get_human(),
        mft_file_size.get_human(),
        total_data_rate.get_human(),
        entries_rate.get::<hertz>().trunc().separate_with_commas()
    );
    Ok(rtn)
}
