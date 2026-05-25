#![allow(
    clippy::borrow_as_ptr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment,
    clippy::cast_sign_loss,
    clippy::undocumented_unsafe_blocks,
    reason = "Windows USN journal IOCTL interop requires raw pointer and integer conversion boilerplate"
)]

use crate::machine::security::encode_wide;
use eyre::Context;
use tracing::debug;
use tracing::debug_span;
use tracing::instrument;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION;
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_MODE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_QUERY_USN_JOURNAL;
use windows::Win32::System::Ioctl::FSCTL_READ_USN_JOURNAL;
use windows::Win32::System::Ioctl::READ_USN_JOURNAL_DATA_V1;
use windows::Win32::System::Ioctl::USN_JOURNAL_DATA_V0;
use windows::Win32::System::Ioctl::USN_REASON_BASIC_INFO_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_CLOSE;
use windows::Win32::System::Ioctl::USN_REASON_COMPRESSION_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_DATA_EXTEND;
use windows::Win32::System::Ioctl::USN_REASON_DATA_OVERWRITE;
use windows::Win32::System::Ioctl::USN_REASON_DATA_TRUNCATION;
use windows::Win32::System::Ioctl::USN_REASON_DESIRED_STORAGE_CLASS_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_EA_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_ENCRYPTION_CHANGE;
pub use windows::Win32::System::Ioctl::USN_REASON_FILE_CREATE;
pub use windows::Win32::System::Ioctl::USN_REASON_FILE_DELETE;
pub use windows::Win32::System::Ioctl::USN_REASON_HARD_LINK_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_INDEXABLE_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_INTEGRITY_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_NAMED_DATA_EXTEND;
use windows::Win32::System::Ioctl::USN_REASON_NAMED_DATA_OVERWRITE;
use windows::Win32::System::Ioctl::USN_REASON_NAMED_DATA_TRUNCATION;
use windows::Win32::System::Ioctl::USN_REASON_OBJECT_ID_CHANGE;
pub use windows::Win32::System::Ioctl::USN_REASON_RENAME_NEW_NAME;
pub use windows::Win32::System::Ioctl::USN_REASON_RENAME_OLD_NAME;
use windows::Win32::System::Ioctl::USN_REASON_REPARSE_POINT_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_SECURITY_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_STREAM_CHANGE;
use windows::Win32::System::Ioctl::USN_REASON_TRANSACTED_CHANGE;
use windows::Win32::System::Ioctl::USN_RECORD_V2;
use windows::Win32::System::Ioctl::USN_RECORD_V3;
use windows::core::PCWSTR;

const JOURNAL_BUFFER_BYTES: usize = 1024 * 1024;
const RELEVANT_REASON_MASK: u32 = USN_REASON_FILE_CREATE
    | USN_REASON_FILE_DELETE
    | USN_REASON_RENAME_OLD_NAME
    | USN_REASON_RENAME_NEW_NAME
    | USN_REASON_HARD_LINK_CHANGE
    | USN_REASON_BASIC_INFO_CHANGE
    | USN_REASON_CLOSE
    | USN_REASON_DATA_OVERWRITE
    | USN_REASON_DATA_EXTEND
    | USN_REASON_DATA_TRUNCATION
    | USN_REASON_COMPRESSION_CHANGE
    | USN_REASON_EA_CHANGE
    | USN_REASON_ENCRYPTION_CHANGE
    | USN_REASON_INDEXABLE_CHANGE
    | USN_REASON_INTEGRITY_CHANGE
    | USN_REASON_NAMED_DATA_OVERWRITE
    | USN_REASON_NAMED_DATA_EXTEND
    | USN_REASON_NAMED_DATA_TRUNCATION
    | USN_REASON_OBJECT_ID_CHANGE
    | USN_REASON_REPARSE_POINT_CHANGE
    | USN_REASON_SECURITY_CHANGE
    | USN_REASON_STREAM_CHANGE
    | USN_REASON_TRANSACTED_CHANGE
    | USN_REASON_DESIRED_STORAGE_CLASS_CHANGE;

#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalCursor {
    pub journal_id: u64,
    pub first_usn: u64,
    pub next_usn: u64,
    pub lowest_valid_usn: u64,
    pub max_usn: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnEvent {
    pub frn: u64,
    pub parent_frn: u64,
    pub usn: u64,
    pub reason: u32,
    pub file_attributes: u32,
    pub name: String,
}

impl UsnEvent {
    #[must_use]
    pub fn is_directory(&self) -> bool {
        self.file_attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
    }

    #[must_use]
    pub fn affects_topology(&self) -> bool {
        self.reason
            & (USN_REASON_FILE_CREATE
                | USN_REASON_FILE_DELETE
                | USN_REASON_RENAME_OLD_NAME
                | USN_REASON_RENAME_NEW_NAME
                | USN_REASON_HARD_LINK_CHANGE)
            != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnReadBatch {
    pub next_usn: u64,
    pub events: Vec<UsnEvent>,
}

#[derive(Debug)]
pub struct VolumeUsnJournal {
    drive_letter: char,
    handle: OwnedHandle,
}

impl VolumeUsnJournal {
    /// # Errors
    ///
    /// Returns an error if the NTFS volume handle cannot be opened.
    #[instrument(level = "debug")]
    pub fn open(drive_letter: char) -> eyre::Result<Self> {
        let volume_path = format!(r"\\.\{drive_letter}:");
        let wide = encode_wide(&volume_path);
        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_GENERIC_READ.0,
                FILE_SHARE_MODE(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0),
                None,
                FILE_CREATION_DISPOSITION(OPEN_EXISTING.0),
                FILE_FLAGS_AND_ATTRIBUTES::default(),
                None,
            )
        }
        .wrap_err_with(|| format!("Failed opening NTFS volume handle for {volume_path}"))?;

        debug!(
            drive = %drive_letter,
            volume_path,
            desired_access = FILE_GENERIC_READ.0,
            "Opened USN journal volume handle"
        );
        Ok(Self {
            drive_letter,
            handle: OwnedHandle(handle),
        })
    }

    /// # Errors
    ///
    /// Returns an error if the current journal metadata cannot be queried.
    #[instrument(level = "debug", skip_all, fields(drive = %self.drive_letter))]
    pub fn query_cursor(&self) -> eyre::Result<JournalCursor> {
        let mut output = USN_JOURNAL_DATA_V0::default();
        let mut bytes_returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.handle.0,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(std::ptr::from_mut(&mut output).cast()),
                size_of::<USN_JOURNAL_DATA_V0>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .wrap_err_with(|| {
            format!(
                "Failed querying USN journal metadata for {}. \
This usually means the volume does not expose an NTFS USN journal or the volume handle was opened with incompatible rights.",
                self.drive_letter,
            )
        })?;

        Ok(JournalCursor {
            journal_id: output.UsnJournalID,
            first_usn: output.FirstUsn as u64,
            next_usn: output.NextUsn as u64,
            lowest_valid_usn: output.LowestValidUsn as u64,
            max_usn: output.MaxUsn as u64,
        })
    }

    /// # Errors
    ///
    /// Returns an error if reading or decoding the USN journal fails.
    #[instrument(level = "debug", skip_all, fields(drive = %self.drive_letter, start_usn))]
    /// # Panics
    ///
    /// Panics if the configured journal read buffer length cannot fit in the
    /// Win32 `u32` length field, which would indicate a fundamentally unsupported build target.
    pub fn read_available_since(
        &self,
        start_usn: u64,
        journal_id: u64,
    ) -> eyre::Result<UsnReadBatch> {
        let mut next_usn = start_usn;
        let mut events = Vec::new();
        let mut iteration = 0usize;

        loop {
            let _span = debug_span!(
                "read_usn_iteration",
                drive = %self.drive_letter,
                iteration,
                next_usn
            )
            .entered();
            iteration += 1;

            let mut read_input = READ_USN_JOURNAL_DATA_V1 {
                StartUsn: next_usn as i64,
                ReasonMask: RELEVANT_REASON_MASK,
                ReturnOnlyOnClose: 0,
                Timeout: 0,
                BytesToWaitFor: 0,
                UsnJournalID: journal_id,
                MinMajorVersion: 2,
                MaxMajorVersion: 3,
            };
            let mut buffer = vec![0u8; JOURNAL_BUFFER_BYTES];
            let mut bytes_returned = 0u32;
            unsafe {
                DeviceIoControl(
                    self.handle.0,
                    FSCTL_READ_USN_JOURNAL,
                    Some(std::ptr::from_mut(&mut read_input).cast()),
                    size_of::<READ_USN_JOURNAL_DATA_V1>() as u32,
                    Some(buffer.as_mut_ptr().cast()),
                    buffer.len().try_into().expect("journal buffer fits in u32"),
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .wrap_err_with(|| {
                format!(
                    "Failed reading USN journal for drive {} from USN {}",
                    self.drive_letter, next_usn
                )
            })?;

            let bytes_returned = bytes_returned as usize;
            if bytes_returned < size_of::<u64>() {
                break;
            }

            let advanced = u64::from_le_bytes(
                buffer[..size_of::<u64>()]
                    .try_into()
                    .expect("cursor prefix is 8 bytes"),
            );
            if advanced == next_usn {
                break;
            }

            parse_records(&buffer[size_of::<u64>()..bytes_returned], &mut events)?;
            next_usn = advanced;
        }

        debug!(
            drive = %self.drive_letter,
            event_count = events.len(),
            next_usn,
            "Read available USN journal events"
        );
        Ok(UsnReadBatch { next_usn, events })
    }
}

fn parse_records(bytes: &[u8], events: &mut Vec<UsnEvent>) -> eyre::Result<()> {
    let mut offset = 0usize;
    while offset + size_of::<u32>() + size_of::<u16>() * 2 <= bytes.len() {
        let record_length = u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("record length prefix fits"),
        ) as usize;
        if record_length == 0 || offset + record_length > bytes.len() {
            eyre::bail!(
                "Corrupt USN journal buffer: invalid record length {} at offset {}",
                record_length,
                offset
            );
        }

        let major_version =
            u16::from_le_bytes(bytes[offset + 4..offset + 6].try_into().expect("u16 slice"));
        match major_version {
            2 => events.push(parse_record_v2(&bytes[offset..offset + record_length])?),
            3 => events.push(parse_record_v3(&bytes[offset..offset + record_length])?),
            _ => {}
        }

        offset += record_length;
    }
    Ok(())
}

fn parse_record_v2(record_bytes: &[u8]) -> eyre::Result<UsnEvent> {
    let record = unsafe { &*(record_bytes.as_ptr().cast::<USN_RECORD_V2>()) };
    Ok(UsnEvent {
        frn: record.FileReferenceNumber,
        parent_frn: record.ParentFileReferenceNumber,
        usn: record.Usn as u64,
        reason: record.Reason,
        file_attributes: record.FileAttributes,
        name: decode_usn_name(
            record_bytes,
            record.FileNameOffset as usize,
            record.FileNameLength as usize,
        )?,
    })
}

fn parse_record_v3(record_bytes: &[u8]) -> eyre::Result<UsnEvent> {
    let record = unsafe { &*(record_bytes.as_ptr().cast::<USN_RECORD_V3>()) };
    Ok(UsnEvent {
        frn: low_u64_from_file_id(record.FileReferenceNumber.Identifier),
        parent_frn: low_u64_from_file_id(record.ParentFileReferenceNumber.Identifier),
        usn: record.Usn as u64,
        reason: record.Reason,
        file_attributes: record.FileAttributes,
        name: decode_usn_name(
            record_bytes,
            record.FileNameOffset as usize,
            record.FileNameLength as usize,
        )?,
    })
}

fn low_u64_from_file_id(identifier: [u8; 16]) -> u64 {
    u64::from_le_bytes(
        identifier[..8]
            .try_into()
            .expect("FILE_ID_128 prefix is always 8 bytes"),
    )
}

fn decode_usn_name(
    record_bytes: &[u8],
    name_offset: usize,
    name_len_bytes: usize,
) -> eyre::Result<String> {
    let name_end = name_offset + name_len_bytes;
    if name_end > record_bytes.len() || !name_len_bytes.is_multiple_of(2) {
        eyre::bail!(
            "Corrupt USN record name payload: offset={} len={} record_len={}",
            name_offset,
            name_len_bytes,
            record_bytes.len()
        );
    }

    let units = record_bytes[name_offset..name_end]
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).wrap_err("USN record filename was not valid UTF-16")
}

#[cfg(test)]
mod tests {
    use super::decode_usn_name;
    use super::low_u64_from_file_id;

    #[test]
    fn file_id_low_bits_roundtrip() {
        let id = [1, 0, 0, 0, 0, 0, 0, 0, 9, 9, 9, 9, 9, 9, 9, 9];
        assert_eq!(low_u64_from_file_id(id), 1);
    }

    #[test]
    fn decode_usn_name_reads_utf16_payload() -> eyre::Result<()> {
        let record = [
            0u8, 0, 0, 0, 0, 0, // ignored prefix
            b'a', 0, b'b', 0, b'c', 0,
        ];
        assert_eq!(decode_usn_name(&record, 6, 6)?, "abc");
        Ok(())
    }
}
