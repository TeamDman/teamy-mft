use std::borrow::Cow;

use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

use crate::windows_utils::storage::DriveLetterPattern;

const DEFAULT_MAXIMUM_SIZE: u64 = 0x8000000;
const DEFAULT_ALLOCATION_DELTA: u64 = 0x1000000;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct FsutilUsnCreateJournalArgs {
    /// Drive letter to enable
    #[facet(args::positional)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Maximum journal size in bytes
    #[facet(args::named, default)]
    pub maximum_size: Option<u64>,

    /// Journal allocation delta in bytes
    #[facet(args::named, default)]
    pub allocation_delta: Option<u64>,

    /// Bypass the machine daemon and create the journal from this process
    #[facet(args::named, default)]
    pub no_daemon: bool,
}

impl FsutilUsnCreateJournalArgs {
    /// # Errors
    ///
    /// Returns an error if the journal cannot be created or queried afterward.
    pub fn invoke(self) -> eyre::Result<()> {
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        let maximum_size = self.maximum_size.unwrap_or(DEFAULT_MAXIMUM_SIZE);
        let allocation_delta = self.allocation_delta.unwrap_or(DEFAULT_ALLOCATION_DELTA);
        if self.no_daemon {
            if !crate::windows_utils::elevation::is_elevated() {
                eyre::bail!(
                    "--no-daemon requires an elevated process. Run from an Administrator shell or omit --no-daemon to send the work to the elevated teamy-mft daemon."
                );
            }
            for drive_letter in drive_letters {
                let status = crate::machine::usn::create_journal(
                    drive_letter,
                    maximum_size,
                    allocation_delta,
                )?;
                print_usn_journal_status(&status);
            }
        } else {
            let config = crate::machine::ipc::load_machine_daemon_client_config()?;
            crate::machine::ipc::ensure_daemon_ready(&config)?;
            for drive_letter in drive_letters {
                let (logs_tx, logs_rx) =
                    vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
                let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
                let status = crate::machine::ipc::create_usn_journal(
                    &config,
                    crate::machine::ipc::CreateUsnJournalRequest {
                        drive_letter,
                        maximum_size,
                        allocation_delta,
                    },
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
