use eyre::Context;
use eyre::eyre;
use std::ops::Deref;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER;
use windows::Win32::System::Ioctl::VOLUME_DISK_EXTENTS;
use windows::core::Owned;

const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

#[derive(Debug)]
pub struct NtfsDriveHandle {
    pub handle: Owned<HANDLE>,
}
impl NtfsDriveHandle {
    /// Ensure the provided handle points to an NTFS volume.
    ///
    /// # Errors
    ///
    /// Returns an error if the device control call indicates the volume is not NTFS.
    pub fn try_new(drive_handle: Owned<HANDLE>) -> eyre::Result<Self> {
        validate_ntfs_filesystem(*drive_handle)?;
        Ok(NtfsDriveHandle {
            handle: drive_handle,
        })
    }
    #[must_use]
    pub fn new_unchecked(drive_handle: Owned<HANDLE>) -> Self {
        NtfsDriveHandle {
            handle: drive_handle,
        }
    }
}
impl TryFrom<Owned<HANDLE>> for NtfsDriveHandle {
    type Error = eyre::Report;

    fn try_from(drive_handle: Owned<HANDLE>) -> Result<Self, Self::Error> {
        Self::try_new(drive_handle)
    }
}
impl Deref for NtfsDriveHandle {
    type Target = Owned<HANDLE>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}
impl AsRef<HANDLE> for NtfsDriveHandle {
    fn as_ref(&self) -> &HANDLE {
        &self.handle
    }
}

/// Validates that the specified drive is using NTFS filesystem
fn validate_ntfs_filesystem(drive_handle: HANDLE) -> eyre::Result<()> {
    let mut volume_data = NTFS_VOLUME_DATA_BUFFER::default();
    let mut bytes_returned = 0u32;
    let buffer_size = u32::try_from(std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>())
        .expect("NTFS_VOLUME_DATA_BUFFER fits in u32");

    // SAFETY: DeviceIoControl requires valid pointers and buffer sizes. The provided struct
    // is stack-allocated, properly aligned, and `buffer_size` matches its size.
    let result = unsafe {
        DeviceIoControl(
            drive_handle,
            FSCTL_GET_NTFS_VOLUME_DATA,
            None,
            0,
            Some((&raw mut volume_data).cast::<std::ffi::c_void>()),
            buffer_size,
            Some(&raw mut bytes_returned),
            None,
        )
    };
    result.wrap_err(eyre!(
        "Drive does not appear to be using NTFS filesystem. FSCTL_GET_NTFS_VOLUME_DATA failed. MFT dumping is only supported on NTFS volumes."
    ))
}

/// Get the disk extents for the volume associated with the drive letter
/// Query the disk extents for the target drive letter.
///
/// # Errors
///
/// Returns an error if opening the drive handle or performing the `DeviceIoControl` call fails.
///
/// # Panics
///
/// Panics if `VOLUME_DISK_EXTENTS` does not fit in `u32` (should not happen).
pub fn get_volume_disk_extents(drive_letter: char) -> eyre::Result<VOLUME_DISK_EXTENTS> {
    use teamy_windows::handle::get_read_only_drive_handle;

    let handle = get_read_only_drive_handle(drive_letter)
        .wrap_err_with(|| format!("Failed to open handle to drive {drive_letter}"))?;

    let mut extents = VOLUME_DISK_EXTENTS::default();
    let mut bytes_returned = 0u32;

    let volume_buffer_size = u32::try_from(std::mem::size_of::<VOLUME_DISK_EXTENTS>())
        .expect("VOLUME_DISK_EXTENTS fits in u32");

    // SAFETY: DeviceIoControl writes into the provided `extents` struct whose size matches `volume_buffer_size`.
    unsafe {
        DeviceIoControl(
            *handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some((&raw mut extents).cast::<std::ffi::c_void>()),
            volume_buffer_size,
            Some(&raw mut bytes_returned),
            None,
        )
        .wrap_err("DeviceIoControl for IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS failed")?;
    };

    if extents.NumberOfDiskExtents != 1 {
        eyre::bail!("Volume spans multiple disks or has complex extents, not supported");
    }

    Ok(extents)
}
