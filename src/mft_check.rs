use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft_process::process_mft_file;
use crate::sync_dir::try_get_sync_dir;
use std::path::PathBuf;
use tracing::debug;

pub fn check_drives(drive_letter_pattern: DriveLetterPattern) -> eyre::Result<()> {
    // Get MFT files from sync dir
    let sync_dir = try_get_sync_dir()?;
    let drive_letters: Vec<char> = drive_letter_pattern.into_drive_letters()?;
    debug!(
        "Pattern {:?} gave drive letters: {:?}",
        drive_letter_pattern, drive_letters
    );
    let mft_files: Vec<(char, PathBuf)> = drive_letters
        .into_iter()
        .map(|d| (d, sync_dir.join(format!("{d}.mft"))))
        .filter(|(_, p)| p.is_file())
        .collect();
    debug!(
        "Checking MFT files: {:#?}",
        mft_files.iter().map(|(_, p)| p).collect::<Vec<_>>()
    );

        let handles: Vec<_> = mft_files
            .into_iter()
            .map(|(drive_letter, mft_file_path)| {
                std::thread::spawn(move || {
                    process_mft_file(&drive_letter.to_string(), &mft_file_path)
                })
            })
            .collect();
        let mut first_err: Option<eyre::Report> = None;
        for h in handles {
            match h.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
                Err(panic) => {
                    let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                        *s
                    } else if let Some(s) = panic.downcast_ref::<String>() {
                        s.as_str()
                    } else {
                        "unknown panic"
                    };
                    return Err(eyre::eyre!("Thread panicked: {msg}"));
                }
            }
        }
        if let Some(e) = first_err {
            return Err(e);
        }
    
    Ok(())
}
