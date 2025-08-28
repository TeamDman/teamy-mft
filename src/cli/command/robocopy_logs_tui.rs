use crate::windows::win_file_content_watch::StartBehaviour;
use crate::windows::win_file_content_watch::watch_file_content;
use arbitrary::Arbitrary;
use clap::Args;
use std::path::PathBuf;
use tracing::info;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct RobocopyLogsTuiArgs {
    /// Path to the robocopy logs text file
    pub robocopy_log_file_path: PathBuf,
}

impl RobocopyLogsTuiArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        info!(
            "Tailing robocopy log (skip start): {}",
            self.robocopy_log_file_path.display()
        );
        let rx = watch_file_content(&self.robocopy_log_file_path, StartBehaviour::SkipStart)?;
        for chunk in rx.iter() {
            print!("{}", String::from_utf8_lossy(&chunk));
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
        Ok(())
    }
}

impl crate::cli::to_args::ToArgs for RobocopyLogsTuiArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        vec![self.robocopy_log_file_path.clone().into()]
    }
}
