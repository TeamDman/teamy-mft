use std::borrow::Cow;

use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

use crate::windows_utils::storage::DriveLetterPattern;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct FsutilUsnQueryJournalArgs {
    /// Drive letter to inspect
    #[facet(args::positional, default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Bypass the machine daemon and query the volume from this process
    #[facet(args::named, default)]
    pub no_daemon: bool,
}

impl FsutilUsnQueryJournalArgs {
    /// # Errors
    ///
    /// Returns an error if the journal query fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        if self.no_daemon {
            if !crate::windows_utils::elevation::is_elevated() {
                eyre::bail!(
                    "--no-daemon requires an elevated process. Run from an Administrator shell or omit --no-daemon to send the work to the elevated teamy-mft daemon."
                );
            }
            for drive_letter in drive_letters {
                let status = crate::machine::usn::query_journal_status(drive_letter)?;
                print_usn_journal_status(&status);
            }
        } else {
            let config = crate::machine::ipc::load_machine_daemon_client_config()?;
            crate::machine::ipc::ensure_daemon_ready(&config)?;
            for drive_letter in drive_letters {
                let (logs_tx, logs_rx) =
                    vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
                let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
                let status = crate::machine::ipc::query_usn_journal(
                    &config,
                    crate::machine::ipc::UsnJournalRequest { drive_letter },
                    logs_tx,
                )?;
                let _ = log_drain.join();
                print_usn_journal_status(&status);
            }
        }
        Ok(())
    }
}

fn print_usn_journal_status(status: &crate::machine::ipc::UsnJournalStatus) {
    println!("usn-drive-letter={}", status.drive_letter);
    println!("usn-journal-active={}", status.active);
    println!(
        "usn-journal-id={}",
        status
            .journal_id
            .map_or_else(|| Cow::from("none"), |id| Cow::from(id.to_string()))
    );
    println!(
        "usn-first-usn={}",
        status
            .first_usn
            .map_or_else(|| Cow::from("none"), |id| Cow::from(id.to_string()))
    );
    println!(
        "usn-next-usn={}",
        status
            .next_usn
            .map_or_else(|| Cow::from("none"), |id| Cow::from(id.to_string()))
    );
    println!(
        "usn-lowest-valid-usn={}",
        status
            .lowest_valid_usn
            .map_or_else(|| Cow::from("none"), |id| Cow::from(id.to_string()))
    );
    println!(
        "usn-max-usn={}",
        status
            .max_usn
            .map_or_else(|| Cow::from("none"), |id| Cow::from(id.to_string()))
    );
    if let Some(reason) = &status.inactive_reason {
        println!("usn-inactive-reason={reason}");
        println!(
            "usn-enable-command=teamy-mft fsutil usn create-journal {}",
            status.drive_letter
        );
    }
}
