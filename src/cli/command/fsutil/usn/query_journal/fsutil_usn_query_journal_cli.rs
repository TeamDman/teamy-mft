use crate::windows_utils::elevation::ensure_elevated;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::borrow::Cow;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct FsutilUsnQueryJournalArgs {
    /// Drive letter to inspect
    #[facet(args::positional, default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Bypass the machine daemon and query the volume from this process
    #[facet(args::named, default)]
    pub no_daemon: bool,

    /// Filter drives by USN journal active state
    #[facet(args::named, default)]
    pub filter: FsutilUsnQueryJournalFilter,
}

#[derive(Default, Facet, Arbitrary, Clone, Copy, PartialEq, Eq, Debug, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum FsutilUsnQueryJournalFilter {
    #[default]
    None,
    Active,
    Inactive,
}

impl FsutilUsnQueryJournalFilter {
    fn matches(self, status: &crate::machine::ipc::UsnJournalStatus) -> bool {
        match self {
            Self::None => true,
            Self::Active => status.active,
            Self::Inactive => !status.active,
        }
    }
}

impl FsutilUsnQueryJournalArgs {
    /// # Errors
    ///
    /// Returns an error if the journal query fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let filter = self.filter;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        if self.no_daemon {
            ensure_elevated()?;
            for drive_letter in drive_letters {
                let status = crate::machine::usn::query_journal_status(drive_letter)?;
                if !filter.matches(&status) {
                    continue;
                }
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
                if !filter.matches(&status) {
                    continue;
                }
                print_usn_journal_status(&status);
            }
        }
        Ok(())
    }
}

fn print_usn_journal_status(status: &crate::machine::ipc::UsnJournalStatus) {
    println!("==============================");
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
