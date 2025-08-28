use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::info;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, GetFileSizeEx, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_SHARE_DELETE, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_END, FILE_BEGIN,
};
use crate::windows::win_strings::EasyPCWSTR;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct RobocopyLogsTuiArgs {
    /// Path to the robocopy logs text file
    pub robocopy_log_file_path: PathBuf,
}

impl RobocopyLogsTuiArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        if !self.robocopy_log_file_path.is_file() {
            eyre::bail!(
                "Log file does not exist: {}",
                self.robocopy_log_file_path.display()
            );
        }
        info!(
            "Tailing (Win32 ReadFile) new robocopy log content (10s): {}",
            self.robocopy_log_file_path.display()
        );

        // Open file via Win32 API with shared access
        let handle = unsafe {
            CreateFileW(
                self.robocopy_log_file_path.as_path().easy_pcwstr()?.as_ref(),
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .with_context(|| {
            format!(
                "Failed to open robocopy log file: {}",
                self.robocopy_log_file_path.display()
            )
        })?;

        // Seek to end to skip existing content
        let mut current_pos: i64 = 0;
        unsafe { SetFilePointerEx(handle, 0, Some(&mut current_pos), FILE_END) }?;

        let start = Instant::now();
        let mut buf = vec![0u8; 64 * 1024];
        while start.elapsed() < Duration::from_secs(10) {
            let mut bytes_read: u32 = 0;
            let read_res = unsafe { ReadFile(handle, Some(buf.as_mut_slice()), Some(&mut bytes_read), None) };
            match read_res {
                Ok(_) => {
                    if bytes_read > 0 {
                        current_pos += bytes_read as i64;
                        print!("{}", String::from_utf8_lossy(&buf[..bytes_read as usize]));
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                    } else {
                        // No new data; check for truncation
                        let mut size: i64 = 0;
                        unsafe { GetFileSizeEx(handle, &mut size) }?;
                        if size < current_pos {
                            unsafe { SetFilePointerEx(handle, 0, Some(&mut current_pos), FILE_BEGIN) }?;
                            current_pos = 0;
                            info!("File truncated; reset to start");
                        } else {
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    }
                }
                Err(e) => {
                    eprintln!("ReadFile error: {e:?}");
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
        info!("Finished 10s tail window");
        Ok(())
    }
}

impl crate::cli::to_args::ToArgs for RobocopyLogsTuiArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        vec![self.robocopy_log_file_path.clone().into()]
    }
}
