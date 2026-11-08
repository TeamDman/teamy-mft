#![warn(clippy::absolute_paths, unused_qualifications)]

use crate::machine::MachineCacheSyncRequest;
use crate::machine::MachineConfig;
use crate::machine::daemon_log::DaemonLogWireEvent;
use crate::machine::daemon_log::spawn_stderr_log_drain;
use crate::machine::ipc;
use crate::sync::SyncPlan;
use crate::sync::sync_path_into_published_overlay;
use crate::sync::sync_path_recursively_into_published_overlay;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use teamy_cancellation::CancellationScope;

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
    pub fn invoke(
        self,
        console_pid: Option<u32>,
        cancellation_scope: &CancellationScope,
    ) -> eyre::Result<()> {
        let plan = self.plan;
        eyre::ensure!(
            !(self.daemon && self.no_daemon),
            "`--daemon` and `--no-daemon` cannot be used together"
        );
        eyre::ensure!(
            !plan.recursive || plan.path.is_some(),
            "`--recursive` requires a target path"
        );

        if self.daemon {
            let config = MachineConfig::load_for_client()?;
            ipc::ensure_daemon_ready(&config)?;
            let (logs_tx, logs_rx) = vox::channel::<DaemonLogWireEvent>();
            let log_drain = spawn_stderr_log_drain(logs_rx);
            let result = ipc::sync(&config, plan, logs_tx);
            let _ = log_drain.join();
            result?;
        } else {
            let sync_dir = MachineConfig::load()?.sync_dir;
            if let Some(path) = plan.path.as_deref() {
                let drive_letter = if plan.recursive {
                    sync_path_recursively_into_published_overlay(&sync_dir, path)?
                } else {
                    sync_path_into_published_overlay(&sync_dir, path)?
                };
                if plan.recursive {
                    println!(
                        "Updated published overlay for drive {drive_letter} with subtree {path}"
                    );
                } else {
                    println!("Updated published overlay for drive {drive_letter} with path {path}");
                }
            } else {
                let drive_letters = plan.drive_letter_pattern.clone().into_drive_letters()?;
                MachineCacheSyncRequest {
                    sync_dir: sync_dir.into_inner(),
                    drive_letters,
                    if_exists: plan.if_exists,
                    parallel: plan.parallel,
                    console_pid,
                }
                .execute_blocking(cancellation_scope)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SyncArgs;
    use crate::cancellation::CancellationToken;
    use crate::sync::SyncPlan;

    #[test]
    fn recursive_sync_requires_target_path() {
        CancellationToken::new()
            .with_scope(|scope| {
                let error = SyncArgs {
                    plan: SyncPlan {
                        recursive: true,
                        ..SyncPlan::default()
                    },
                    ..SyncArgs::default()
                }
                .invoke(None, &scope)
                .expect_err("recursive sync without a path should fail before execution");

                assert!(
                    error
                        .to_string()
                        .contains("`--recursive` requires a target path")
                );
                eyre::Ok(())
            })
            .unwrap();
    }
}
