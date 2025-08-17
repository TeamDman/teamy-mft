use crate::windows::win_handles::AutoClosingHandle;
use eyre::bail;
use windows::Win32::Storage::FileSystem::FILE_BEGIN;
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::Storage::FileSystem::SetFilePointerEx;

impl AutoClosingHandle {
    pub fn try_read_exact(&self, offset: i64, buf: &mut [u8]) -> eyre::Result<()> {
        // Seek
        unsafe {
            SetFilePointerEx(**self, offset, None, FILE_BEGIN)?;
        }

        // Read
        let mut bytes_read = 0;
        unsafe {
            ReadFile(**self, Some(buf), Some(&mut bytes_read), None)?;
        }
        if bytes_read != buf.len() as u32 {
            bail!(
                "Failed to read from handle, expected to read {} bytes, got {}",
                buf.len(),
                bytes_read
            );
        }
        Ok(())
    }
}
