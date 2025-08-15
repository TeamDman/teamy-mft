use crate::drive_letter_pattern::DriveLetterPattern;
use crate::sync_dir::try_get_sync_dir;
use eyre::Result;
use eyre::WrapErr;
use eyre::bail;
use memmap2::Mmap;
use mft::fast_entry;
use mft::fast_fixup;
use mft::path_resolve;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::debug;
use tracing::info;
use uom::si::f64::Information;
use uom::si::f64::InformationRate;
use uom::si::f64::Time;
use uom::si::information::byte;
use uom::si::time::second;

/// Statistics and timing information for processing a single MFT file.
pub struct MftProcessStats {
    pub path: PathBuf,
    pub size_bytes: usize,
    pub entry_size: usize,
    pub entry_count: usize,
    pub fixups_applied: u64,
    pub fixups_already: u64,
    pub fixups_invalid: u64,
    pub filename_attrs: usize,
    pub resolved_paths: usize,
    pub dur_fixups: Duration,
    pub dur_scan: Duration,
    pub dur_resolve: Duration,
    pub sample_paths: Vec<PathBuf>,
}

pub fn check_drives(drive_letter_pattern: DriveLetterPattern) -> eyre::Result<()> {
    // Get MFT files from sync dir
    let sync_dir = try_get_sync_dir()?;
    let drive_letters: Vec<char> = drive_letter_pattern.into_drive_letters()?;
    debug!(
        "Pattern {:?} gave drive letters: {:?}",
        drive_letter_pattern, drive_letters
    );
    let mft_files: Vec<PathBuf> = drive_letters
        .into_iter()
        .map(|d| sync_dir.join(format!("{d}.mft")))
        .filter(|p| p.is_file())
        .collect();
    debug!("Checking MFT files: {:#?}", mft_files);

    let timeout = std::time::Duration::from_secs(10);
    for mft_file_path in &mft_files {
        process_mft_file(mft_file_path, timeout, 20)?;
    }
    Ok(())
}

/// Overall high-level processing of an MFT file: mmap -> copy -> fixups -> extract names -> resolve paths.
pub fn process_mft_file(
    mft_file_path: &Path,
    timeout: Duration,
    sample_limit: usize,
) -> Result<MftProcessStats> {
    info!("Processing MFT file: {}", mft_file_path.display());

    let start = std::time::Instant::now();

    // open file
    let file = std::fs::File::open(mft_file_path)
        .with_context(|| format!("Failed to open {}", mft_file_path.display()))?;
    debug!("Opened MFT file: {}", mft_file_path.display());

    // file size
    let file_size_bytes = file
        .metadata()
        .with_context(|| format!("Failed to get metadata for {}", mft_file_path.display()))?
        .len() as usize;
    let file_size = Information::new::<byte>(file_size_bytes as f64);
    if file_size_bytes < 1024 {
        eyre::bail!("MFT file too small: {}", mft_file_path.display());
    }

    // mmap
    debug!("Memory-mapping {}", file_size.get_human());
    let mmap_start = Instant::now();
    let mmap = unsafe { Mmap::map(&file) }
        .with_context(|| format!("Failed to memory-map {}", mft_file_path.display()))?;
    let bytes: &[u8] = &mmap;
    let mmap_elapsed = Time::new::<second>(mmap_start.elapsed().as_secs_f64());
    let mmap_rate = InformationRate::from(file_size / mmap_elapsed);
    debug!(
        "Memory-mapped {} in {}, {}/s",
        file_size.get_human(),
        mmap_elapsed.get_human(),
        mmap_rate.get_human()
    );
    assert_eq!(file_size_bytes, bytes.len());

    // detect entry size
    let entry_size = fast_fixup::detect_entry_size(&bytes[0..1024]).unwrap_or(1024) as usize;
    if entry_size == 0 || bytes.len() % entry_size != 0 {
        eyre::bail!("Unaligned entry size for {}", mft_file_path.display());
    }
    let entry_count = bytes.len() / entry_size;

    // copy bytes (allows in-place fixup)
    let mut owned = Vec::with_capacity(bytes.len());
    owned.extend_from_slice(bytes);

    // apply fixups (parallel)
    let t_fix = Instant::now();
    let stats = fast_fixup::apply_fixups_parallel(&mut owned, entry_size);
    let dur_fixups = t_fix.elapsed();

    // collect filename attributes (parallel)
    let t_scan = Instant::now();
    let (names, per_entry) = fast_entry::par_collect_filenames(&owned, entry_size);
    let dur_scan = t_scan.elapsed();

    // resolve paths (sequential baseline)
    let t_resolve = Instant::now();
    let paths = path_resolve::resolve_paths_simple(&names, &per_entry);
    let dur_resolve = t_resolve.elapsed();
    let resolved_paths = paths.iter().flatten().count();

    // sample
    let sample_paths: Vec<PathBuf> = paths.iter().flatten().take(sample_limit).cloned().collect();

    let rtn = MftProcessStats {
        path: mft_file_path.to_path_buf(),
        size_bytes: bytes.len(),
        entry_size,
        entry_count,
        fixups_applied: stats.applied,
        fixups_already: stats.already_applied,
        fixups_invalid: stats.invalid,
        filename_attrs: names.len(),
        resolved_paths,
        dur_fixups,
        dur_scan,
        dur_resolve,
        sample_paths,
    };
    info!(
        "MFT {}: size={}MB entries={} entry_size={} fixups(applied/already/invalid)={}/{}/{} names={} resolved={} timings(fix/scan/resolve)={:.3}/{:.3}/{:.3}s",
        rtn.path.display(),
        (rtn.size_bytes as f64) / (1024.0 * 1024.0),
        rtn.entry_count,
        rtn.entry_size,
        rtn.fixups_applied,
        rtn.fixups_already,
        rtn.fixups_invalid,
        rtn.filename_attrs,
        rtn.resolved_paths,
        rtn.dur_fixups.as_secs_f64(),
        rtn.dur_scan.as_secs_f64(),
        rtn.dur_resolve.as_secs_f64()
    );
    for p in &rtn.sample_paths {
        info!("PATH: {}", p.display());
    }

    let elapsed = start.elapsed();
    if elapsed > timeout {
        bail!(
            "Taking too long to check entries in {}: {}",
            mft_file_path.display(),
            elapsed.as_secs_f64()
        );
    }
    Ok(rtn)
}
