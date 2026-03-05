use eyre::Context;
use eyre::eyre;
use std::ops::Deref;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;
use windows::Win32::System::Ioctl::NTFS_VOLUME_DATA_BUFFER;
use windows::core::Owned;

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
