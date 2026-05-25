use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::time::SystemTime;
use teamy_windows::storage::DriveLetterPattern;

/// Show freshness information for cached `.mft` and `.mft_search_index` files.
#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
#[facet(rename_all = "kebab-case")]
pub struct StatusArgs {
    /// Drive letter pattern to inspect (e.g., `*`, `C`, `CD`, `C,D`).
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Show per-drive artifact paths and timestamps.
    #[facet(args::named, default)]
    pub verbose: bool,
}

#[derive(Debug)]
struct DaemonStatusSummary {
    ping: Option<crate::machine::ipc::PingResponse>,
    compatibility: Option<crate::machine::ipc::DaemonCompatibility>,
    runtime_status: Option<crate::machine::ipc::StatusResponse>,
    warning: Option<String>,
}

impl StatusArgs {
    /// # Errors
    ///
    /// Returns an error if drive letters cannot be resolved or cached file metadata cannot be read.
    pub fn invoke(self) -> eyre::Result<()> {
        let machine_status =
            crate::machine::status::load_machine_status(&self.drive_letter_pattern)?;
        let daemon_status = load_daemon_status_summary(&machine_status)?;
        print_machine_summary(&machine_status, self.verbose);
        print_daemon_summary(&daemon_status);

        if machine_status.config.is_none() {
            return Ok(());
        }
        let status = crate::status::TeamyMftStatus::load(&self.drive_letter_pattern)?;
        let now = SystemTime::now();

        print_cache_summary(&status, now, self.verbose);

        Ok(())
    }
}

fn load_daemon_status_summary(
    machine_status: &crate::machine::status::MachineStatus,
) -> eyre::Result<DaemonStatusSummary> {
    let Some(config) = machine_status.config.as_ref() else {
        return Ok(DaemonStatusSummary {
            ping: None,
            compatibility: None,
            runtime_status: None,
            warning: None,
        });
    };

    if !matches!(
        machine_status.service_state,
        crate::machine::service::WindowsServiceState::Running
            | crate::machine::service::WindowsServiceState::StartPending
    ) {
        return Ok(DaemonStatusSummary {
            ping: None,
            compatibility: None,
            runtime_status: None,
            warning: None,
        });
    }

    let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogEvent>();
    let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
    let ping_response = crate::machine::ipc::ping(config, logs_tx);
    let _ = log_drain.join();

    let ping = match ping_response? {
        Ok(ping) => ping,
        Err(error) => {
            return Ok(DaemonStatusSummary {
                ping: None,
                compatibility: None,
                runtime_status: None,
                warning: Some(format!("daemon ping failed: {}", error.message)),
            });
        }
    };

    let compatibility = crate::machine::ipc::daemon_compatibility(&ping);
    let warning = if compatibility.rpc_compat_matches {
        if compatibility.is_fully_matching() {
            None
        } else {
            Some(String::from(
                "daemon build metadata differs from the current CLI; reinstall or restart the daemon to fully resync versions",
            ))
        }
    } else {
        Some(String::from(
            "daemon RPC compatibility version differs from the current CLI; reinstall or restart the daemon before relying on live RPC features",
        ))
    };

    let runtime_status = if compatibility.rpc_compat_matches {
        let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogEvent>();
        let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
        let response =
            crate::machine::ipc::status(config, crate::machine::ipc::StatusRequest, logs_tx);
        let _ = log_drain.join();
        response?.ok()
    } else {
        None
    };

    Ok(DaemonStatusSummary {
        ping: Some(ping),
        compatibility: Some(compatibility),
        runtime_status,
        warning,
    })
}

fn print_daemon_runtime_summary(status: &crate::machine::ipc::StatusResponse) {
    println!(
        "machine-daemon-loaded-drive-count={}",
        status.loaded_drive_letters.len()
    );
    println!(
        "machine-daemon-degraded-drive-count={}",
        status.degraded_drives.len()
    );
    println!(
        "machine-daemon-buffered-log-count={}",
        status.buffered_log_count
    );
    for degraded_drive in &status.degraded_drives {
        println!(
            "machine-daemon-drive-{}-degraded={}",
            degraded_drive.drive_letter, degraded_drive.message
        );
    }
}

fn print_daemon_summary(summary: &DaemonStatusSummary) {
    println!("machine-daemon-cli-app-version={}", crate::APP_SEMVER);
    println!(
        "machine-daemon-cli-git-revision={}",
        crate::APP_GIT_REVISION
    );
    println!(
        "machine-daemon-cli-rpc-compat-version={}",
        crate::DAEMON_RPC_COMPAT_VERSION
    );
    println!("machine-daemon-reachable={}", summary.ping.is_some());

    if let Some(ping) = &summary.ping {
        println!("machine-daemon-service-name={}", ping.service_name);
        println!("machine-daemon-app-version={}", ping.build.app_version);
        println!("machine-daemon-git-revision={}", ping.build.git_revision);
        println!(
            "machine-daemon-rpc-compat-version={}",
            ping.build.rpc_compat_version
        );
    }

    if let Some(compatibility) = &summary.compatibility {
        println!(
            "machine-daemon-rpc-compat-match={}",
            compatibility.rpc_compat_matches
        );
        println!(
            "machine-daemon-app-version-match={}",
            compatibility.app_version_matches
        );
        println!(
            "machine-daemon-git-revision-match={}",
            compatibility.git_revision_matches
        );
        println!(
            "machine-daemon-build-fully-matching={}",
            compatibility.is_fully_matching()
        );
    }

    if let Some(warning) = &summary.warning {
        println!("machine-daemon-warning={warning}");
    }

    if let Some(status) = &summary.runtime_status {
        print_daemon_runtime_summary(status);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Verbose status output is intentionally emitted as a flat summary block"
)]
fn print_machine_summary(machine_status: &crate::machine::status::MachineStatus, verbose: bool) {
    println!("machine-managed={}", machine_status.config.is_some());
    println!(
        "machine-service-state={}",
        format_service_state(machine_status.service_state)
    );
    println!(
        "machine-current-user-sid={}",
        machine_status
            .current_user_sid
            .as_deref()
            .unwrap_or("unknown")
    );

    if let Some(config) = &machine_status.config {
        println!("machine-service-name={}", config.service_name);
        println!("machine-cache-root={}", config.cache_root.display());
        println!("machine-owner-sid={}", config.owner_sid);
        println!("machine-owner-access={}", machine_status.owner_access);
        println!("machine-drive-count={}", machine_status.drives.len());
        println!(
            "machine-published-drive-count={}",
            machine_status
                .drives
                .iter()
                .filter(|drive| drive.base_index_modified_at.is_some() && drive.checkpoint.is_some())
                .count()
        );
        for drive in &machine_status.drives {
            println!(
                "machine-drive-{}-published={}",
                drive.drive_letter,
                drive.base_index_modified_at.is_some() && drive.checkpoint.is_some()
            );
            println!(
                "machine-drive-{}-overlay-present={}",
                drive.drive_letter,
                drive.overlay_index_modified_at.is_some()
            );
            println!(
                "machine-drive-{}-snapshot-usn={}",
                drive.drive_letter,
                format_optional_u64(
                    drive
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.snapshot_usn)
                )
            );
            println!(
                "machine-drive-{}-last-usn={}",
                drive.drive_letter,
                format_optional_u64(
                    drive
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.last_usn)
                )
            );
            if verbose {
                println!(
                    "machine-drive-{}-mft-path={}",
                    drive.drive_letter,
                    drive.mft_path.display()
                );
                println!(
                    "machine-drive-{}-mft-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.mft_modified_at)
                );
                println!(
                    "machine-drive-{}-base-index-path={}",
                    drive.drive_letter,
                    drive.base_index_path.display()
                );
                println!(
                    "machine-drive-{}-base-index-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.base_index_modified_at)
                );
                println!(
                    "machine-drive-{}-overlay-index-path={}",
                    drive.drive_letter,
                    drive.overlay_index_path.display()
                );
                println!(
                    "machine-drive-{}-overlay-index-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.overlay_index_modified_at)
                );
                println!(
                    "machine-drive-{}-checkpoint-path={}",
                    drive.drive_letter,
                    drive.checkpoint_path.display()
                );
                println!(
                    "machine-drive-{}-checkpoint-modified-at={}",
                    drive.drive_letter,
                    crate::status::format_optional_system_time(drive.checkpoint_modified_at)
                );
                println!(
                    "machine-drive-{}-journal-id={}",
                    drive.drive_letter,
                    format_optional_u64(
                        drive
                            .checkpoint
                            .as_ref()
                            .and_then(|checkpoint| checkpoint.journal_id)
                    )
                );
            }
        }
    }
}

fn print_cache_summary(status: &crate::status::TeamyMftStatus, now: SystemTime, verbose: bool) {
    println!(
        "machine-cache-query-ready-drive-count={}",
        status.query_ready_drive_count()
    );
    println!(
        "machine-cache-oldest-query-ready-age={}",
        crate::status::format_optional_duration(status.oldest_query_ready_age(now))
    );
    println!(
        "machine-cache-newest-query-ready-age={}",
        crate::status::format_optional_duration(status.newest_query_ready_age(now))
    );

    if !verbose {
        return;
    }

    println!(
        "machine-cache-oldest-query-ready-at={}",
        crate::status::format_optional_system_time(status.oldest_query_ready_at())
    );
    println!(
        "machine-cache-newest-query-ready-at={}",
        crate::status::format_optional_system_time(status.newest_query_ready_at())
    );

    for drive in &status.drives {
        println!(
            "machine-cache-drive-{}-query-ready={}",
            drive.drive_letter,
            drive.is_query_ready()
        );
        println!(
            "machine-cache-drive-{}-query-ready-age={}",
            drive.drive_letter,
            crate::status::format_optional_duration(drive.query_ready_age(now))
        );
        println!(
            "machine-cache-drive-{}-mft-path={}",
            drive.drive_letter,
            drive.mft_path.display()
        );
        println!(
            "machine-cache-drive-{}-mft-modified-at={}",
            drive.drive_letter,
            crate::status::format_optional_system_time(drive.mft_modified_at)
        );
        println!(
            "machine-cache-drive-{}-index-path={}",
            drive.drive_letter,
            drive.index_path.display()
        );
        println!(
            "machine-cache-drive-{}-index-modified-at={}",
            drive.drive_letter,
            crate::status::format_optional_system_time(drive.index_modified_at)
        );
    }
}

fn format_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| String::from("none"), |value| value.to_string())
}

fn format_service_state(value: crate::machine::service::WindowsServiceState) -> &'static str {
    match value {
        crate::machine::service::WindowsServiceState::Missing => "missing",
        crate::machine::service::WindowsServiceState::Stopped => "stopped",
        crate::machine::service::WindowsServiceState::StartPending => "start-pending",
        crate::machine::service::WindowsServiceState::Running => "running",
        crate::machine::service::WindowsServiceState::Unknown(_) => "unknown",
    }
}
