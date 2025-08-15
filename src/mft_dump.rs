use crate::windows::win_handles::get_drive_handle;
use eyre::Context;
use eyre::eyre;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::mem::size_of;
use std::path::Path;
use tracing::info;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::LUID;
use windows::Win32::Security::AdjustTokenPrivileges;
use windows::Win32::Security::LookupPrivilegeValueW;
use windows::Win32::Security::SE_BACKUP_NAME;
use windows::Win32::Security::SE_PRIVILEGE_ENABLED;
use windows::Win32::Security::SE_RESTORE_NAME;
use windows::Win32::Security::SE_SECURITY_NAME;
use windows::Win32::Security::TOKEN_ADJUST_PRIVILEGES;
use windows::Win32::Security::TOKEN_PRIVILEGES;
use windows::Win32::Security::TOKEN_QUERY;
use windows::Win32::Storage::FileSystem::FILE_BEGIN;
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::Storage::FileSystem::SetFilePointerEx;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::OpenProcessToken;

/// Dumps the MFT to the specified file path
pub async fn dump_mft_to_file<P: AsRef<Path>>(
    output_path: P,
    drive_letter: char,
) -> eyre::Result<()> {
    let output_path = output_path.as_ref();

    // Use the provided drive letter
    let drive_letter = drive_letter.to_uppercase().next().unwrap_or('C');

    // Open the drive handle once and reuse it
    let drive_handle = get_drive_handle(drive_letter)
        .wrap_err_with(|| format!("Failed to open handle to drive {drive_letter}"))?;

    // Validate that the drive is using NTFS filesystem
    info!("Validating filesystem type for drive {}...", drive_letter);
    validate_ntfs_filesystem(*drive_handle, drive_letter)
        .wrap_err_with(|| format!("NTFS validation failed for drive {drive_letter}"))?;

    info!("Reading MFT data from drive {}...", drive_letter);
    // Synchronously read MFT data to avoid holding HANDLE across await points
    let mft_data = read_mft_data(*drive_handle)?;

    info!("Writing MFT data to '{}'...", output_path.display());
    write_mft_to_file(&mft_data, output_path)?;

    info!(
        "Successfully dumped MFT ({}) to '{}'",
        humansize::format_size(mft_data.len(), humansize::DECIMAL),
        output_path.display()
    );

    Ok(())
}

/// Validates that the specified drive is using NTFS filesystem
fn validate_ntfs_filesystem(drive_handle: HANDLE, drive_letter: char) -> eyre::Result<()> {
    let mut volume_data = NTFS_VOLUME_DATA_BUFFER::default();
    let mut bytes_returned = 0u32;

    let result = unsafe {
        DeviceIoControl(
            drive_handle,
            FSCTL_GET_NTFS_VOLUME_DATA,
            None,
            0,
            Some(&mut volume_data as *mut _ as *mut _),
            size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };
    result.wrap_err(eyre!(
        "Drive {} does not appear to be using NTFS filesystem. FSCTL_GET_NTFS_VOLUME_DATA failed. MFT dumping is only supported on NTFS volumes.",
        drive_letter
    ))
}

/// Reads the raw MFT data by parsing the MFT's own record and following its data runs
fn read_mft_data(drive_handle: HANDLE) -> eyre::Result<Vec<u8>> {
    info!("Reading MFT using proper data runs parsing approach");
    read_mft_from_volume_with_dataruns(drive_handle)
}

/// Reads the MFT by parsing the boot sector and following data runs properly
pub(crate) fn read_mft_from_volume_with_dataruns(drive_handle: HANDLE) -> eyre::Result<Vec<u8>> {
    // Step 1: Read the boot sector to get NTFS parameters
    let boot_sector = read_boot_sector(drive_handle)?;

    let bytes_per_cluster =
        boot_sector.bytes_per_sector as u64 * boot_sector.sectors_per_cluster as u64;
    let mft_location = boot_sector.mft_cluster_number * bytes_per_cluster;

    info!(
        "NTFS Boot Sector: bytes/sector={}, sectors/cluster={}, MFT cluster={}, MFT offset={} bytes",
        boot_sector.bytes_per_sector,
        boot_sector.sectors_per_cluster,
        boot_sector.mft_cluster_number,
        mft_location
    );

    // Step 2: Read the MFT's own record (record 0)
    let mft_record = read_mft_record(drive_handle, mft_location, 0)?;

    // Step 3: Parse the MFT record to find the DATA attribute (0x80)
    let data_runs = parse_mft_record_for_data_attribute(&mft_record)?;

    // Step 4: Follow the data runs to read the complete MFT
    read_mft_using_data_runs_blocking(drive_handle, &data_runs, bytes_per_cluster)
}

/// NTFS boot sector information
#[derive(Debug)]
pub(crate) struct NtfsBootSector {
    pub(crate) bytes_per_sector: u16,
    pub(crate) sectors_per_cluster: u8,
    pub(crate) mft_cluster_number: u64,
}

/// Reads and parses the NTFS boot sector
pub(crate) fn read_boot_sector(drive_handle: HANDLE) -> eyre::Result<NtfsBootSector> {
    // Seek to the beginning of the drive
    unsafe {
        SetFilePointerEx(drive_handle, 0, None, FILE_BEGIN)
            .wrap_err_with(|| "Failed to seek to boot sector")?;
    }

    // Read the boot sector (512 bytes)
    let mut boot_sector = vec![0u8; 512];
    let mut bytes_read = 0u32;
    unsafe {
        ReadFile(
            drive_handle,
            Some(boot_sector.as_mut_slice()),
            Some(&mut bytes_read),
            None,
        )
        .wrap_err_with(|| "Failed to read boot sector")?;
    }

    if bytes_read != 512 {
        return Err(eyre!(
            "Failed to read complete boot sector: got {} bytes",
            bytes_read
        ));
    }

    // Parse relevant fields from the boot sector
    let bytes_per_sector = u16::from_le_bytes([boot_sector[0x0b], boot_sector[0x0c]]);
    let sectors_per_cluster = boot_sector[0x0d];
    let mft_cluster_number = u64::from_le_bytes([
        boot_sector[0x30],
        boot_sector[0x31],
        boot_sector[0x32],
        boot_sector[0x33],
        boot_sector[0x34],
        boot_sector[0x35],
        boot_sector[0x36],
        boot_sector[0x37],
    ]);

    Ok(NtfsBootSector {
        bytes_per_sector,
        sectors_per_cluster,
        mft_cluster_number,
    })
}

/// Reads a specific MFT record
pub(crate) fn read_mft_record(
    drive_handle: HANDLE,
    mft_location: u64,
    record_number: u64,
) -> eyre::Result<Vec<u8>> {
    // MFT records are typically 1024 bytes each
    const MFT_RECORD_SIZE: u64 = 1024;
    let record_offset = mft_location + (record_number * MFT_RECORD_SIZE);

    // Seek to the record
    unsafe {
        SetFilePointerEx(drive_handle, record_offset as i64, None, FILE_BEGIN)
            .wrap_err_with(|| format!("Failed to seek to MFT record {record_number}"))?;
    }

    // Read the record
    let mut record = vec![0u8; MFT_RECORD_SIZE as usize];
    let mut bytes_read = 0u32;
    unsafe {
        ReadFile(
            drive_handle,
            Some(record.as_mut_slice()),
            Some(&mut bytes_read),
            None,
        )
        .wrap_err_with(|| format!("Failed to read MFT record {record_number}"))?;
    }

    if bytes_read != MFT_RECORD_SIZE as u32 {
        return Err(eyre!(
            "Failed to read complete MFT record: got {} bytes",
            bytes_read
        ));
    }

    // Verify this is a valid MFT record by checking the signature
    if &record[0..4] != b"FILE" {
        return Err(eyre!(
            "Invalid MFT record signature: expected 'FILE', got '{}'",
            String::from_utf8_lossy(&record[0..4])
        ));
    }

    Ok(record)
}

/// Data run information
#[derive(Debug)]
pub(crate) struct DataRun {
    pub(crate) length: u64,  // Length in clusters
    pub(crate) cluster: i64, // Cluster offset (can be negative for relative positioning)
}

/// Parses an MFT record to extract data runs from the DATA attribute (0x80)
pub(crate) fn parse_mft_record_for_data_attribute(record: &[u8]) -> eyre::Result<Vec<DataRun>> {
    // Get the offset to the first attribute (typically at offset 20)
    let attr_offset = u16::from_le_bytes([record[20], record[21]]) as usize;
    let mut read_ptr = attr_offset;

    while read_ptr < record.len() {
        // Read attribute header
        if read_ptr + 8 > record.len() {
            break;
        }

        let attr_type = u32::from_le_bytes([
            record[read_ptr],
            record[read_ptr + 1],
            record[read_ptr + 2],
            record[read_ptr + 3],
        ]);

        // Check for end marker
        if attr_type == 0xffffffff {
            break;
        }

        let attr_length = u32::from_le_bytes([
            record[read_ptr + 4],
            record[read_ptr + 5],
            record[read_ptr + 6],
            record[read_ptr + 7],
        ]) as usize;

        if attr_length == 0 {
            break;
        }

        // Check if this is the DATA attribute (0x80)
        if attr_type == 0x80 {
            // Check if it's non-resident (byte at offset 8 should be != 0)
            if read_ptr + 8 < record.len() && record[read_ptr + 8] != 0 {
                // Get the data runs offset (at offset 32 from attribute start)
                if read_ptr + 34 <= record.len() {
                    let run_offset =
                        u16::from_le_bytes([record[read_ptr + 32], record[read_ptr + 33]]) as usize;

                    let data_runs_start = read_ptr + run_offset;
                    let data_runs_end = read_ptr + attr_length;

                    if data_runs_start < data_runs_end && data_runs_end <= record.len() {
                        return decode_data_runs(&record[data_runs_start..data_runs_end]);
                    }
                }
            }
        }

        read_ptr += attr_length;
    }

    Err(eyre!("Could not find DATA attribute (0x80) in MFT record"))
}

/// Decodes NTFS data runs
fn decode_data_runs(data_runs: &[u8]) -> eyre::Result<Vec<DataRun>> {
    let mut runs = Vec::new();
    let mut decode_pos = 0;

    while decode_pos < data_runs.len() {
        let header = data_runs[decode_pos];

        // End of data runs
        if header == 0 {
            break;
        }

        let offset_bytes = (header & 0xf0) >> 4;
        let length_bytes = header & 0x0f;

        if offset_bytes == 0 || length_bytes == 0 {
            break;
        }

        decode_pos += 1;

        // Read length (little-endian)
        if decode_pos + length_bytes as usize > data_runs.len() {
            break;
        }

        let mut length = 0u64;
        for i in 0..length_bytes {
            length |= (data_runs[decode_pos + i as usize] as u64) << (i * 8);
        }
        decode_pos += length_bytes as usize;

        // Read offset (little-endian, signed)
        if decode_pos + offset_bytes as usize > data_runs.len() {
            break;
        }

        let mut cluster = 0i64;
        for i in 0..offset_bytes {
            cluster |= (data_runs[decode_pos + i as usize] as i64) << (i * 8);
        }

        // Handle sign extension for the offset
        if offset_bytes > 0 {
            let sign_bit = 1i64 << (offset_bytes * 8 - 1);
            if cluster & sign_bit != 0 {
                cluster |= !((1i64 << (offset_bytes * 8)) - 1);
            }
        }

        decode_pos += offset_bytes as usize;

        runs.push(DataRun { length, cluster });
    }

    Ok(runs)
}

/// Blocking helper that reads the complete MFT using the parsed data runs
fn read_mft_using_data_runs_blocking(
    drive_handle: HANDLE,
    data_runs: &[DataRun],
    bytes_per_cluster: u64,
) -> eyre::Result<Vec<u8>> {
    let mut mft_data = Vec::new();
    let mut current_cluster = 0i64;

    info!("Found {} data runs for MFT", data_runs.len());

    for (i, run) in data_runs.iter().enumerate() {
        // Calculate absolute cluster position
        current_cluster += run.cluster;

        let byte_offset = current_cluster as u64 * bytes_per_cluster;
        let byte_length = run.length * bytes_per_cluster;

        info!(
            "Data run {}: cluster {} (offset {}), length {} clusters ({})",
            i + 1,
            current_cluster,
            humansize::format_size(byte_offset, humansize::DECIMAL),
            run.length,
            humansize::format_size(byte_length, humansize::DECIMAL)
        );

        // Seek to the run location
        unsafe {
            SetFilePointerEx(drive_handle, byte_offset as i64, None, FILE_BEGIN).wrap_err_with(
                || {
                    format!(
                        "Failed to seek to data run {} at offset {}",
                        i + 1,
                        byte_offset
                    )
                },
            )?;
        }

        // Read the run data
        let mut run_data = vec![0u8; byte_length as usize];
        let mut total_read = 0;
        let mut offset = 0;

        while offset < byte_length {
            let remaining = byte_length - offset;
            let chunk_size = remaining.min(1024 * 1024) as usize; // Read in 1MB chunks

            let mut bytes_read = 0u32;
            unsafe {
                ReadFile(
                    drive_handle,
                    Some(&mut run_data[offset as usize..offset as usize + chunk_size]),
                    Some(&mut bytes_read),
                    None,
                )
                .wrap_err_with(|| {
                    format!("Failed to read data run {} at offset {}", i + 1, offset)
                })?;
            }

            if bytes_read == 0 {
                break;
            }

            offset += bytes_read as u64;
            total_read += bytes_read as u64;
        }

        run_data.truncate(total_read as usize);
        mft_data.extend_from_slice(&run_data);

        info!(
            "Read {} from data run {}",
            humansize::format_size(total_read, humansize::DECIMAL),
            i + 1
        );
    }

    info!(
        "Successfully read complete MFT: {}",
        humansize::format_size(mft_data.len(), humansize::DECIMAL)
    );

    Ok(mft_data)
}

/// Writes the MFT data to the specified file
pub(crate) fn write_mft_to_file(mft_data: &[u8], output_path: &Path) -> eyre::Result<()> {
    let mut file = if output_path.exists() {
        // If file exists and we got here, overwrite_existing must be true
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(output_path)
            .wrap_err_with(|| {
                format!("Failed to open file for writing: {}", output_path.display())
            })?
    } else {
        // Create new file
        File::create(output_path)
            .wrap_err_with(|| format!("Failed to create file: {}", output_path.display()))?
    };

    file.write_all(mft_data).wrap_err_with(|| {
        format!(
            "Failed to write MFT data to file: {}",
            output_path.display()
        )
    })?;

    file.flush()
        .wrap_err_with(|| format!("Failed to flush file: {}", output_path.display()))?;

    Ok(())
}

/// Enables backup and security privileges for the current process
pub fn enable_backup_privileges() -> eyre::Result<()> {
    use std::mem::size_of;

    unsafe {
        // Get current process token
        let mut token = windows::Win32::Foundation::HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .wrap_err_with(|| "Failed to open process token")?;

        // Enable multiple privileges that might be needed
        let privileges_to_enable = [SE_BACKUP_NAME, SE_RESTORE_NAME, SE_SECURITY_NAME];

        for privilege_name in &privileges_to_enable {
            // Look up the privilege LUID
            let mut luid = LUID::default();
            if LookupPrivilegeValueW(None, *privilege_name, &mut luid).is_ok() {
                // Set up the privilege structure
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [windows::Win32::Security::LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };

                // Adjust token privileges
                let _ = AdjustTokenPrivileges(
                    token,
                    false,
                    Some(&privileges),
                    size_of::<TOKEN_PRIVILEGES>() as u32,
                    None,
                    None,
                );
            }
        }

        // Close token handle
        windows::Win32::Foundation::CloseHandle(token)
            .wrap_err_with(|| "Failed to close token handle")?;

        info!("Successfully enabled backup privileges");
        Ok(())
    }
}

// --- added: blocking wrapper to avoid moving HANDLEs across threads ---

/// Blocking wrapper: open drive handle in current thread, run blocking read/parsing logic, write output file.
/// This avoids moving HANDLE values across threads.
pub fn dump_mft_to_file_blocking<P: AsRef<Path>>(
    output_path: P,
    drive_letter: char,
) -> eyre::Result<()> {
    let output_path = output_path.as_ref();

    // normalize drive letter
    let drive_letter = drive_letter.to_uppercase().next().unwrap_or('C');

    // open handle inside this thread
    let drive_handle = get_drive_handle(drive_letter)
        .wrap_err_with(|| format!("Failed to open handle to drive {}", drive_letter))?;

    // validate NTFS
    validate_ntfs_filesystem(*drive_handle, drive_letter)
        .wrap_err_with(|| format!("NTFS validation failed for drive {}", drive_letter))?;

    // read MFT using existing blocking parser that returns Vec<u8>
    let mft_data = read_mft_from_volume_with_dataruns(*drive_handle)
        .wrap_err_with(|| format!("Failed to read MFT for drive {}", drive_letter))?;

    // write to file
    write_mft_to_file(&mft_data, output_path).wrap_err_with(|| {
        format!(
            "Failed to write MFT for drive {} -> {}",
            drive_letter,
            output_path.display()
        )
    })?;

    Ok(())
}
