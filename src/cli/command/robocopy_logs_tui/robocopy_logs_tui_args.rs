use crate::cli::to_args::ToArgs;
use crate::robocopy::robocopy_log_parser::RobocopyLogParser;
use crate::robocopy::robocopy_log_parser::RobocopyParseAdvance;
use arbitrary::Arbitrary;
use clap::Args;
use std::path::PathBuf;
use teamy_windows::file::WatchConfig;
use teamy_windows::file::watch_file_content;
use tracing::info;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct RobocopyLogsTuiArgs {
    /// Path to the robocopy logs text file
    pub robocopy_log_file_path: PathBuf,
    /// Avoids printing content before the current time
    #[arg(long, default_value_t = true)]
    pub skip_to_present: bool,
}

impl RobocopyLogsTuiArgs {
    /// Display robocopy logs in a TUI.
    ///
    /// # Errors
    ///
    /// Returns an error if the log file cannot be read or parsed.
    pub fn invoke(self) -> eyre::Result<()> {
        info!(
            "Tailing robocopy log (skip start): {}",
            self.robocopy_log_file_path.display()
        );

        let mut parser = RobocopyLogParser::new();

        // We must start from the beginning even if skipping to latest logs
        // This is to ensure the parser is able to advance its state machine correctly
        let rx = watch_file_content(WatchConfig::new_from_start(self.robocopy_log_file_path))?;

        info!("Waiting for first chunk");
        let first_chunk = rx.recv()?;

        info!("Sending first chunk to parser");
        let s = String::from_utf8_lossy(&first_chunk);
        parser.accept(&s);

        if self.skip_to_present {
            info!("Skipping to present...");

            info!("Draining the buffer");
            while let Ok(chunk) = rx.try_recv() {
                let s = String::from_utf8_lossy(&chunk);
                parser.accept(&s);
            }

            info!("Advancing the parser");
            loop {
                match parser.advance()? {
                    RobocopyParseAdvance::NeedMoreData => {
                        info!("All caught up!");
                        break;
                    }
                    RobocopyParseAdvance::Header(_robocopy_header) => {
                        // info!("Skipped header: {robocopy_header}");
                    }
                    RobocopyParseAdvance::LogEntry(_robocopy_log_entry) => {
                        // info!("Skipped log entry: {robocopy_log_entry:?}");
                    }
                }
            }
        }

        for chunk in &rx {
            let s = String::from_utf8_lossy(&chunk);
            parser.accept(&s);
            loop {
                match parser.advance()? {
                    RobocopyParseAdvance::NeedMoreData => {
                        println!("Need more data...");
                        break;
                    }
                    RobocopyParseAdvance::Header(h) => {
                        println!("[HEADER]\n{h}");
                    }
                    RobocopyParseAdvance::LogEntry(e) => {
                        println!("[ENTRY] {e:?}");
                    }
                }
            }
        }
        Ok(())
    }
}

impl ToArgs for RobocopyLogsTuiArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        vec![self.robocopy_log_file_path.clone().into()]
    }
}
