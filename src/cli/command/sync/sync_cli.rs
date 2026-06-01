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
        if self.no_daemon {
            let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
            let drive_letters = plan.drive_letter_pattern.clone().into_drive_letters()?;
            let drive_infos = crate::sync::resolve_drive_infos_in_dir_for_letters(
                &sync_dir,
                drive_letters.iter().copied(),
            )?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(crate::sync::execute_sync(drive_infos, &plan.if_exists))?;
        } else {
            let config = crate::machine::ipc::load_machine_daemon_client_config()?;
            crate::machine::ipc::ensure_daemon_ready(&config)?;
            let (logs_tx, logs_rx) =
                vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
            let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
            crate::machine::ipc::sync(&config, plan, logs_tx)?;
            let _ = log_drain.join();
        }

        Ok(())
    }
}
