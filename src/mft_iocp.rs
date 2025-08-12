use crate::mft_dump::parse_mft_record_for_data_attribute;
use crate::mft_dump::read_boot_sector;
use crate::mft_dump::read_mft_record;
use crate::mft_dump::write_mft_to_file;
use crate::windows::win_strings::EasyPCWSTR;
use eyre::WrapErr;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::CloseHandle;
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
const MAX_IN_FLIGHT_IO: usize = 256; // tuning knob

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
pub fn read_mft_iocp<P: AsRef<Path>>(drive_letter: char, output_path: P) -> eyre::Result<()> {
    let drive_letter = drive_letter.to_ascii_uppercase();
    let volume_path = format!(r"\\.\{}:", drive_letter);
    let volume_path = volume_path
        .as_str()
        .easy_pcwstr()
        .wrap_err("Failed to convert volume path to PCWSTR")?;

    unsafe {
        // Open blocking handle for boot sector & MFT record parsing
        let blocking_handle = CreateFileW(
            &volume_path,
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?;

        let boot = read_boot_sector(blocking_handle).wrap_err("Failed to read boot sector")?;
        let bytes_per_cluster = boot.bytes_per_sector as u64 * boot.sectors_per_cluster as u64;
        let mft_location = boot.mft_cluster_number * bytes_per_cluster;

        let mft_record = read_mft_record(blocking_handle, mft_location, 0)
            .wrap_err("Failed to read MFT record")?;
        let data_runs = parse_mft_record_for_data_attribute(&mft_record)
            .wrap_err("Failed to parse data runs from MFT record")?;

        // compute absolute offsets (on disk), and total logical size (in stream bytes)
        let mut runs_abs = Vec::new();
        let mut current_cluster: i64 = 0;
        let mut total_bytes: u64 = 0;
        for run in &data_runs {
            current_cluster = current_cluster.wrapping_add(run.cluster);
            let disk_offset = current_cluster as u64 * bytes_per_cluster; // absolute on disk
            let run_bytes = run.length * bytes_per_cluster; // bytes in MFT stream
            runs_abs.push((disk_offset, run_bytes));
            total_bytes = total_bytes.saturating_add(run_bytes);
        }

        let _ = CloseHandle(blocking_handle);

        // Open overlapped-capable handle
        let overlapped_handle = CreateFileW(
            &volume_path,
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            None,
        )?;

        // Create IOCP and associate handle
        let cp = CreateIoCompletionPort(overlapped_handle, None, 0, 0)?;

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
                    overlapped_handle,
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
                            cp,
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
                                        dest_lock[start..end]
                                            .copy_from_slice(&boxed_req.buffer[..copy_len]);
                                    }
                                    remaining.fetch_sub(1, Ordering::SeqCst);
                                    in_flight -= 1;
                                }
                            }
                            Err(_) => {
                                if lp_overlapped.is_null() {
                                    let _ = CloseHandle(overlapped_handle);
                                    let _ = CloseHandle(cp);
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
                cp,
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
                            dest_lock[start..end]
                                .copy_from_slice(&boxed_req.buffer[..copy_len]);
                        }
                        remaining.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                Err(_) => {
                    if lp_overlapped.is_null() {
                        let _ = CloseHandle(overlapped_handle);
                        let _ = CloseHandle(cp);
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
            write_mft_to_file(&dest_guard, output_path.as_ref())
                .wrap_err("Failed to write MFT output file")?;
        }

        let _ = CloseHandle(overlapped_handle);
        let _ = CloseHandle(cp);

        Ok(())
    } // unsafe block
}
