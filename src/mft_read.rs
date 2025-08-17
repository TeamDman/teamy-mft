use crate::mft::mft_record_attribute::MftRecordAttribute;
use crate::mft::mft_record::MftRecord;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::mft::ntfs_boot_sector::NtfsBootSector;
use crate::ntfs::ntfs_drive_handle::NtfsDriveHandle;
use crate::windows::win_handles::AutoClosingHandle;
use crate::windows::win_handles::get_drive_handle;
use crate::windows::win_strings::EasyPCWSTR;
use eyre::WrapErr;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::IO::CreateIoCompletionPort;
use windows::Win32::System::IO::GetQueuedCompletionStatus;
use windows::Win32::System::IO::OVERLAPPED;

const CHUNK_SIZE: u64 = 1024 * 1024; // 1 MiB
const MAX_IN_FLIGHT_IO: usize = 16; // tuning knob

#[repr(C)]
struct ReadRequest {
    overlapped: OVERLAPPED, // must be first for pointer cast from OVERLAPPED*
    buffer: Vec<u8>,
    file_offset: u64, // absolute disk offset
    dest_offset: u64, // logical offset within the MFT stream
    length: usize,
}

impl ReadRequest {
    fn new(file_offset: u64, dest_offset: u64, length: usize) -> Box<Self> {
        let buffer = vec![0u8; length];
        let overlapped = OVERLAPPED::default();
        Box::new(ReadRequest {
            overlapped,
            buffer,
            file_offset,
            dest_offset,
            length,
        })
    }
}

/// Read the complete MFT using IOCP overlapped reads.
/// drive_letter: 'C', 'D', ...
/// output_path: file path to write final MFT blob
pub fn read_mft(drive_letter: char, output_path: impl AsRef<Path>) -> eyre::Result<()> {
    let drive_letter = drive_letter.to_ascii_uppercase();
    let volume_path = format!(r"\\.\{drive_letter}:");
    let volume_path = volume_path
        .as_str()
        .easy_pcwstr()
        .wrap_err("Failed to convert volume path to PCWSTR")?;

    unsafe {
        // Open blocking handle for boot sector & MFT record parsing
        let drive_handle: NtfsDriveHandle = get_drive_handle(drive_letter)
            .wrap_err_with(|| format!("Failed to open handle to drive {drive_letter}"))?
            .try_into()
            .wrap_err_with(|| {
                format!(
                    "Failed to convert drive handle for drive {drive_letter} to NtfsDriveHandle"
                )
            })?;

        let boot_sector = NtfsBootSector::try_from_handle(&drive_handle)?;
        let dollar_mft_record = MftRecord::try_from_handle(
            &drive_handle,
            boot_sector.mft_location() + MftRecordNumber::DOLLAR_MFT,
        )?;
        // Gather all non-resident $DATA runlists (could be multiple segments if attribute list used).
        let mut decoded_runs = Vec::new();
        for attr in dollar_mft_record.iter_raw_attributes() {
            if attr.get_attr_type() == MftRecordAttribute::TYPE_DOLLAR_DATA && attr.get_is_non_resident() {
                if let Some(x80) = attr.as_x80() {
                    if let Some(runlist) = x80.get_data_run_list()? {
                        for run_res in runlist.iter() { decoded_runs.push(run_res?); }
                    }
                }
            }
        }
        if decoded_runs.is_empty() { eyre::bail!("No non-resident $DATA runs found in $MFT record"); }
        // Convert relative runs (with optional sparse) into absolute cluster extents list (skip sparse).
    let mut runs_abs = Vec::new();
        let bytes_per_cluster = boot_sector.bytes_per_cluster();
        let mut total_bytes: u64 = 0;
        for run in &decoded_runs {
            let lcn = if let Some(lcn) = run.lcn_start { lcn } else { // sparse: extend logical size only
                total_bytes = total_bytes.saturating_add(run.length_clusters * bytes_per_cluster);
                continue;
            };
            let disk_offset = lcn * bytes_per_cluster;
            let run_bytes = run.length_clusters * bytes_per_cluster;
            runs_abs.push((disk_offset, run_bytes));
            total_bytes = total_bytes.saturating_add(run_bytes);
        }
        // TODO: If sparse extents appear inside $MFT they logically represent gaps (rare); currently skipped.

        drop(drive_handle);

        // Open overlapped-capable handle
        let overlapped_handle: AutoClosingHandle = CreateFileW(
            &volume_path,
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            None,
        )?
        .into();

        // Create IOCP and associate handle
        let completion_port: AutoClosingHandle =
            CreateIoCompletionPort(*overlapped_handle, None, 0, 0)?.into();

        // destination buffer guarded by Mutex; acceptable since writes are large and not CPU bound
        let dest = Arc::new(Mutex::new(vec![0u8; total_bytes as usize]));
        let remaining = Arc::new(AtomicUsize::new(0));

        // schedule reads across runs in CHUNK_SIZE pieces
        let mut in_flight = 0usize;
        let mut dest_base = 0u64; // logical base inside the MFT stream for the current run
        for (run_disk_offset, run_len) in runs_abs {
            let mut offset = 0u64;
            while offset < run_len {
                let this_len = std::cmp::min(CHUNK_SIZE, run_len - offset) as usize;
                let file_offset = run_disk_offset + offset; // absolute on disk
                let dest_offset = dest_base + offset; // logical in-stream position

                let raw_ptr = Box::into_raw(ReadRequest::new(file_offset, dest_offset, this_len));
                let req_ref: &mut ReadRequest = &mut *raw_ptr;

                // set overlapped offset low/high for disk position
                let lo = (file_offset & 0xffff_ffff) as u32;
                let hi = ((file_offset >> 32) & 0xffff_ffff) as u32;
                req_ref.overlapped.Anonymous.Anonymous.Offset = lo;
                req_ref.overlapped.Anonymous.Anonymous.OffsetHigh = hi;

                let overlapped_ptr: *mut OVERLAPPED = &mut req_ref.overlapped;

                // call ReadFile (overlapped)
                let _ = ReadFile(
                    *overlapped_handle,
                    Some(&mut req_ref.buffer[..this_len]),
                    None,
                    Some(overlapped_ptr),
                );
                // We don't inspect return value; completion arrives on IOCP.

                remaining.fetch_add(1, Ordering::SeqCst);
                offset += this_len as u64;

                in_flight += 1;
                if in_flight >= MAX_IN_FLIGHT_IO {
                    // drain until in_flight decreases below threshold
                    while in_flight >= MAX_IN_FLIGHT_IO {
                        let mut bytes_transferred: u32 = 0;
                        let mut completion_key: usize = 0;
                        let mut lp_overlapped: *mut OVERLAPPED = null_mut();

                        let res = GetQueuedCompletionStatus(
                            *completion_port,
                            &mut bytes_transferred as *mut u32,
                            &mut completion_key as *mut usize,
                            &mut lp_overlapped as *mut *mut OVERLAPPED,
                            u32::MAX,
                        );

                        match res {
                            Ok(()) => {
                                if !lp_overlapped.is_null() {
                                    let req_ptr = lp_overlapped as *mut ReadRequest;
                                    let boxed_req = Box::from_raw(req_ptr);
                                    let copy_len =
                                        (bytes_transferred as usize).min(boxed_req.length);
                                    if copy_len > 0 {
                                        let mut dest_lock = dest.lock().unwrap();
                                        let start = boxed_req.dest_offset as usize;
                                        let end = start + copy_len;
                                        dest_lock[start..end]
                                            .copy_from_slice(&boxed_req.buffer[..copy_len]);
                                    }
                                    remaining.fetch_sub(1, Ordering::SeqCst);
                                    in_flight -= 1;
                                }
                            }
                            Err(_) => {
                                if lp_overlapped.is_null() {
                                    return Err(eyre::eyre!(
                                        "GetQueuedCompletionStatus failed and no overlapped provided"
                                    ));
                                } else {
                                    let req_ptr = lp_overlapped as *mut ReadRequest;
                                    let boxed_req = Box::from_raw(req_ptr);
                                    tracing::error!(
                                        "IOCP: read failed for disk offset {} (dest offset {})",
                                        boxed_req.file_offset,
                                        boxed_req.dest_offset
                                    );
                                    remaining.fetch_sub(1, Ordering::SeqCst);
                                    in_flight -= 1;
                                }
                            }
                        }
                    } // drain inner loop
                }
            } // while offset < run_len
            dest_base += run_len; // advance logical base for next run
        } // for each run

        // Drain remaining completions until none outstanding
        while remaining.load(Ordering::SeqCst) > 0 {
            let mut bytes_transferred: u32 = 0;
            let mut completion_key: usize = 0;
            let mut lp_overlapped: *mut OVERLAPPED = null_mut();

            let res = GetQueuedCompletionStatus(
                *completion_port,
                &mut bytes_transferred as *mut u32,
                &mut completion_key as *mut usize,
                &mut lp_overlapped as *mut *mut OVERLAPPED,
                u32::MAX,
            );
            match res {
                Ok(()) => {
                    if !lp_overlapped.is_null() {
                        let req_ptr = lp_overlapped as *mut ReadRequest;
                        let boxed_req = Box::from_raw(req_ptr);
                        let copy_len = (bytes_transferred as usize).min(boxed_req.length);
                        if copy_len > 0 {
                            let mut dest_lock = dest.lock().unwrap();
                            let start = boxed_req.dest_offset as usize;
                            let end = start + copy_len;
                            dest_lock[start..end].copy_from_slice(&boxed_req.buffer[..copy_len]);
                        }
                        remaining.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                Err(_) => {
                    if lp_overlapped.is_null() {
                        return Err(eyre::eyre!(
                            "GetQueuedCompletionStatus failed and no overlapped provided"
                        ));
                    } else {
                        let req_ptr = lp_overlapped as *mut ReadRequest;
                        let boxed_req = Box::from_raw(req_ptr);
                        tracing::error!(
                            "IOCP: read failed for disk offset {} (dest offset {})",
                            boxed_req.file_offset,
                            boxed_req.dest_offset
                        );
                        remaining.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            }
        }

        // All reads complete — write to file
        {
            let dest_guard = dest.lock().unwrap();
            std::fs::write(&output_path, dest_guard.as_slice())
                .wrap_err("Failed to write MFT output file")?;
        }

        Ok(())
    } // unsafe block
}
