use crate::sync::SyncPlan;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, PartialEq, Debug, Arbitrary, Default)]
pub struct SyncArgs {
    #[facet(flatten)]
    pub plan: SyncPlan,

    /// Bypass the machine daemon and run sync work directly in this process
    #[facet(args::named, default)]
    pub no_daemon: bool,

    /// Ask the machine daemon to run sync work
    #[facet(args::named, default)]
    pub daemon: bool,
}

impl SyncArgs {
    /// Sync MFT data from drives.
    ///
    /// # Errors
    ///
    /// Returns an error if the machine daemon is not installed, cannot be started,
    /// or rejects the sync request.
    pub fn invoke(self) -> eyre::Result<()> {
        let plan = self.plan;
        eyre::ensure!(
            !(self.daemon && self.no_daemon),
            "`--daemon` and `--no-daemon` cannot be used together"
        );

        if self.daemon {
            let config = crate::machine::ipc::load_machine_daemon_client_config()?;
            crate::machine::ipc::ensure_daemon_ready(&config)?;
            let (logs_tx, logs_rx) =
                vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
            let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
            crate::machine::ipc::sync(&config, plan, logs_tx)?;
            let _ = log_drain.join();
        } else {
            let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
            if let Some(path) = plan.path.as_deref() {
                let drive_letter = crate::sync::sync_path_into_published_overlay(&sync_dir, path)?;
                println!("Updated published overlay for drive {drive_letter} with path {path}");
            } else {
                let drive_letters = plan.drive_letter_pattern.clone().into_drive_letters()?;
                crate::machine::daemon::sync_machine_cache(
                    &sync_dir,
                    &drive_letters,
                    plan.if_exists,
                )?;
            }
        }

        Ok(())
    }
}
