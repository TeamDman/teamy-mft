use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::borrow::Cow;
use std::time::SystemTime;

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

    /// Bypass the machine daemon and inspect published files directly from this process
    #[facet(args::named, default)]
    pub no_daemon: bool,

    /// Ask the machine daemon for live status
    #[facet(args::named, default)]
    pub daemon: bool,

    /// Allow falling back to direct local inspection if the daemon cannot be reached
    #[facet(args::named, default)]
    pub allow_fallback: bool,
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
        eyre::ensure!(
            !(self.daemon && self.no_daemon),
            "`--daemon` and `--no-daemon` cannot be used together"
        );

        let mut machine_status =
            crate::machine::status::load_machine_status(&self.drive_letter_pattern)?;
        let daemon_status = load_daemon_status_summary(
            &self.drive_letter_pattern,
            self.daemon,
            self.allow_fallback,
        )?;
        if daemon_status.ping.is_some() {
            machine_status.service_state = crate::machine::service::WindowsServiceState::Running;
        }
        let include_direct_drive_details =
            !self.daemon && self.verbose && crate::windows_utils::elevation::is_elevated();
        print_machine_summary(&machine_status, self.verbose, include_direct_drive_details);
        print_daemon_summary(&daemon_status, self.verbose);
        let now = SystemTime::now();
        print_cache_summary(
            &machine_status,
            daemon_status.runtime_status.as_ref(),
            now,
            self.verbose,
        );

        Ok(())
    }
}

fn load_daemon_status_summary(
    drive_letter_pattern: &DriveLetterPattern,
    daemon: bool,
    allow_fallback: bool,
) -> eyre::Result<DaemonStatusSummary> {
    if !daemon {
        return Ok(DaemonStatusSummary {
            ping: None,
            compatibility: None,
            runtime_status: None,
            warning: None,
        });
    }
    let config = crate::machine::ipc::load_machine_daemon_client_config()?;
    let ready_daemon = match crate::machine::ipc::ensure_daemon_ready(&config) {
        Ok(ready_daemon) => ready_daemon,
        Err(error) if allow_fallback => {
            return Ok(DaemonStatusSummary {
                ping: None,
                compatibility: None,
                runtime_status: None,
                warning: Some(format!("daemon readiness failed: {error}")),
            });
        }
        Err(error) => return Err(error),
    };
    let ping = ready_daemon.ping;
    let compatibility = ready_daemon.compatibility;
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
        let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogWireEvent>();
        let log_drain = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);
        let response = crate::machine::ipc::status(
            &config,
            crate::machine::ipc::StatusRequest {
                drive_letters: drive_letter_pattern.clone().into_drive_letters()?,
            },
            logs_tx,
        );
        drop(log_drain);
        match response {
            Ok(status) => Some(status),
            Err(error) if allow_fallback => {
                return Ok(DaemonStatusSummary {
                    ping: Some(ping),
                    compatibility: Some(compatibility),
                    runtime_status: None,
                    warning: Some(format!("daemon status failed: {error}")),
                });
            }
            Err(error) => return Err(error),
        }
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
        "machine-daemon-loading-drive-count={}",
        status.loading_drive_letters.len()
    );
    println!(
        "machine-daemon-snapshot-only-drive-count={}",
        status.snapshot_only_drive_letters.len()
    );
    println!(
        "machine-daemon-degraded-drive-count={}",
        status.degraded_drives.len()
    );
    println!(
        "machine-daemon-active-job-count={}",
        status.active_job_count
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
    for drive_letter in &status.snapshot_only_drive_letters {
        println!("machine-daemon-drive-{drive_letter}-snapshot-only=true");
    }
}

fn print_daemon_summary(summary: &DaemonStatusSummary, verbose: bool) {
    println!("machine-daemon-reachable={}", summary.ping.is_some());

    if let Some(ping) = &summary.ping
        && verbose
    {
        println!("machine-daemon-service-name={}", ping.service_name);
        println!("machine-daemon-app-version={}", ping.build.app_version);
        println!("machine-daemon-git-revision={}", ping.build.git_revision);
        println!("machine-daemon-build-unix-ms={}", ping.build.build_unix_ms);
        println!(
            "machine-daemon-rpc-compat-version={}",
            ping.build.rpc_compat_version
        );
    }

    if let Some(compatibility) = &summary.compatibility {
        println!(
            "machine-daemon-build-fully-matching={}",
            compatibility.is_fully_matching()
        );
        if verbose || !compatibility.is_fully_matching() {
            println!("machine-daemon-cli-app-version={}", crate::APP_SEMVER);
            println!(
                "machine-daemon-cli-git-revision={}",
                crate::APP_GIT_REVISION
            );
            println!(
                "machine-daemon-cli-build-unix-ms={}",
                crate::APP_BUILD_UNIX_MS
            );
            println!(
                "machine-daemon-cli-rpc-compat-version={}",
                crate::DAEMON_RPC_COMPAT_VERSION
            );
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
                "machine-daemon-build-unix-ms-match={}",
                compatibility.build_unix_ms_matches
            );
        }
    }

    if let Some(warning) = &summary.warning {
        println!("machine-daemon-warning={warning}");
    }

    if verbose && let Some(status) = &summary.runtime_status {
        print_daemon_runtime_summary(status);
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Verbose status output is intentionally emitted as a flat summary block"
)]
fn print_machine_summary(
    machine_status: &crate::machine::status::MachineStatus,
    verbose: bool,
    include_direct_drive_details: bool,
) {
    println!("machine-managed={}", machine_status.config.is_some());
    println!(
        "machine-service-state={}",
        match machine_status.service_state {
            crate::machine::service::WindowsServiceState::Missing => "missing",
            crate::machine::service::WindowsServiceState::Stopped => "stopped",
            crate::machine::service::WindowsServiceState::StartPending => "start-pending",
            crate::machine::service::WindowsServiceState::Running => "running",
            crate::machine::service::WindowsServiceState::Unknown(_) => "unknown",
        }
    );
    if verbose {
        println!(
            "machine-current-user-sid={}",
            machine_status
                .current_user_sid
                .as_deref()
                .unwrap_or("unknown")
        );
    }
    if let Some(config_warning) = &machine_status.config_warning {
        println!("machine-config-warning={config_warning}");
    }

    if let Some(config) = &machine_status.config {
        println!("machine-cache-root={}", config.sync_dir.display());
        print_protection_summary(config);
        if verbose {
            println!("machine-service-name={}", config.service_name);
            println!("machine-owner-sid={}", config.owner_sid);
            println!("machine-owner-access={}", machine_status.owner_access);
        }
    }

    if include_direct_drive_details && let Some(_config) = &machine_status.config {
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
                drive
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.snapshot_usn)
                    .map_or_else(|| Cow::from("none"), |value| Cow::from(value.to_string()))
            );
            println!(
                "machine-drive-{}-last-usn={}",
                drive.drive_letter,
                drive
                    .checkpoint
                    .as_ref()
                    .and_then(|checkpoint| checkpoint.last_usn)
                    .map_or_else(|| Cow::from("none"), |value| Cow::from(value.to_string()))
            );
            if let Some(warning) = &drive.warning {
                println!("machine-drive-{}-warning={warning}", drive.drive_letter);
            }
            if verbose {
                println!(
                    "machine-drive-{}-mft-path={}",
                    drive.drive_letter,
                    drive.mft_path.display()
                );
                println!(
                    "machine-drive-{}-mft-modified-at={}",
                    drive.drive_letter,
                    drive.mft_modified_at.map_or_else(
                        || Cow::from("none"),
                        |value| Cow::from(
                            chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339()
                        )
                    )
                );
                println!(
                    "machine-drive-{}-base-index-path={}",
                    drive.drive_letter,
                    drive.base_index_path.display()
                );
                println!(
                    "machine-drive-{}-base-index-modified-at={}",
                    drive.drive_letter,
                    drive.base_index_modified_at.map_or_else(
                        || Cow::from("none"),
                        |value| Cow::from(
                            chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339()
                        )
                    )
                );
                println!(
                    "machine-drive-{}-overlay-index-path={}",
                    drive.drive_letter,
                    drive.overlay_index_path.display()
                );
                println!(
                    "machine-drive-{}-overlay-index-modified-at={}",
                    drive.drive_letter,
                    drive.overlay_index_modified_at.map_or_else(
                        || Cow::from("none"),
                        |value| Cow::from(
                            chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339()
                        )
                    )
                );
                println!(
                    "machine-drive-{}-checkpoint-path={}",
                    drive.drive_letter,
                    drive.checkpoint_path.display()
                );
                println!(
                    "machine-drive-{}-checkpoint-modified-at={}",
                    drive.drive_letter,
                    drive.checkpoint_modified_at.map_or_else(
                        || Cow::from("none"),
                        |value| Cow::from(
                            chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339()
                        )
                    )
                );
                println!(
                    "machine-drive-{}-journal-id={}",
                    drive.drive_letter,
                    drive
                        .checkpoint
                        .as_ref()
                        .and_then(|checkpoint| checkpoint.journal_id)
                        .map_or_else(|| Cow::from("none"), |value| Cow::from(value.to_string()))
                );
            }
        }
    }
}

fn print_protection_summary(config: &crate::machine::config::MachineConfig) {
    match crate::machine::security::query_path_protection_status(
        &config.sync_dir,
        &config.owner_sid,
    ) {
        Ok(status) => {
            crate::machine::security::warn_if_path_protection_disabled(&config.sync_dir, &status);
            crate::machine::security::print_path_protection_status(&status);
        }
        Err(error) => {
            println!("machine-protection-enabled=unknown");
            println!("machine-protection-warning=failed to inspect protection status: {error}");
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "status output intentionally prints a flat machine-readable cache report"
)]
fn print_cache_summary(
    machine_status: &crate::machine::status::MachineStatus,
    daemon_status: Option<&crate::machine::ipc::StatusResponse>,
    now: SystemTime,
    verbose: bool,
) {
    if let Some(daemon_status) = daemon_status {
        print_cache_summary_from_daemon(daemon_status, now, verbose);
        return;
    }

    let query_ready_drives = machine_status
        .drives
        .iter()
        .filter(|drive| drive.mft_modified_at.is_some() && drive.base_index_modified_at.is_some())
        .collect::<Vec<_>>();
    println!("machine-drive-count={}", machine_status.drives.len());
    println!("machine-published-drive-count={}", query_ready_drives.len());
    let oldest_query_ready_at = query_ready_drives
        .iter()
        .filter_map(
            |drive| match (drive.mft_modified_at, drive.base_index_modified_at) {
                (Some(mft_modified_at), Some(index_modified_at)) => {
                    Some(mft_modified_at.min(index_modified_at))
                }
                _ => None,
            },
        )
        .min();
    let newest_query_ready_at = query_ready_drives
        .iter()
        .filter_map(
            |drive| match (drive.mft_modified_at, drive.base_index_modified_at) {
                (Some(mft_modified_at), Some(index_modified_at)) => {
                    Some(mft_modified_at.min(index_modified_at))
                }
                _ => None,
            },
        )
        .max();

    println!(
        "machine-cache-query-ready-drive-count={}",
        query_ready_drives.len()
    );
    println!(
        "machine-cache-oldest-query-ready-age={}",
        oldest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| humantime::format_duration(
                now.duration_since(value)
                    .unwrap_or(std::time::Duration::ZERO)
            )
            .to_string()
            .into()
        )
    );
    println!(
        "machine-cache-newest-query-ready-age={}",
        newest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| humantime::format_duration(
                now.duration_since(value)
                    .unwrap_or(std::time::Duration::ZERO)
            )
            .to_string()
            .into()
        )
    );

    if !verbose {
        return;
    }

    println!(
        "machine-cache-oldest-query-ready-at={}",
        oldest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| Cow::from(chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339())
        )
    );
    println!(
        "machine-cache-newest-query-ready-at={}",
        newest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| Cow::from(chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339())
        )
    );

    for drive in &machine_status.drives {
        let query_ready_at = match (drive.mft_modified_at, drive.base_index_modified_at) {
            (Some(mft_modified_at), Some(index_modified_at)) => {
                Some(mft_modified_at.min(index_modified_at))
            }
            _ => None,
        };
        println!(
            "machine-cache-drive-{}-query-ready={}",
            drive.drive_letter,
            query_ready_at.is_some()
        );
        println!(
            "machine-cache-drive-{}-query-ready-age={}",
            drive.drive_letter,
            query_ready_at.map_or_else(
                || Cow::from("none"),
                |value| humantime::format_duration(
                    now.duration_since(value)
                        .unwrap_or(std::time::Duration::ZERO)
                )
                .to_string()
                .into()
            )
        );
        println!(
            "machine-cache-drive-{}-mft-path={}",
            drive.drive_letter,
            drive.mft_path.display()
        );
        println!(
            "machine-cache-drive-{}-mft-modified-at={}",
            drive.drive_letter,
            drive.mft_modified_at.map_or_else(
                || Cow::from("none"),
                |value| Cow::from(chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339())
            )
        );
        println!(
            "machine-cache-drive-{}-index-path={}",
            drive.drive_letter,
            drive.base_index_path.display()
        );
        println!(
            "machine-cache-drive-{}-index-modified-at={}",
            drive.drive_letter,
            drive.base_index_modified_at.map_or_else(
                || Cow::from("none"),
                |value| Cow::from(chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339())
            )
        );
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "Verbose daemon-backed cache status is intentionally emitted as a flat summary block"
)]
fn print_cache_summary_from_daemon(
    daemon_status: &crate::machine::ipc::StatusResponse,
    now: SystemTime,
    verbose: bool,
) {
    let query_ready_drives = daemon_status
        .published_drives
        .iter()
        .filter(|drive| {
            drive.mft_modified_at_unix_ms.is_some()
                && drive.base_index_modified_at_unix_ms.is_some()
        })
        .collect::<Vec<_>>();
    let oldest_query_ready_at = query_ready_drives
        .iter()
        .filter_map(|drive| {
            match (
                drive
                    .mft_modified_at_unix_ms
                    .map(|value| std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)),
                drive
                    .base_index_modified_at_unix_ms
                    .map(|value| std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)),
            ) {
                (Some(mft_modified_at), Some(index_modified_at)) => {
                    Some(mft_modified_at.min(index_modified_at))
                }
                _ => None,
            }
        })
        .min();
    let newest_query_ready_at = query_ready_drives
        .iter()
        .filter_map(|drive| {
            match (
                drive
                    .mft_modified_at_unix_ms
                    .map(|value| std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)),
                drive
                    .base_index_modified_at_unix_ms
                    .map(|value| std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)),
            ) {
                (Some(mft_modified_at), Some(index_modified_at)) => {
                    Some(mft_modified_at.min(index_modified_at))
                }
                _ => None,
            }
        })
        .max();

    println!(
        "machine-drive-count={}",
        daemon_status.published_drives.len()
    );
    println!(
        "machine-published-drive-count={}",
        daemon_status.published_drives.len()
    );
    println!(
        "machine-cache-query-ready-drive-count={}",
        query_ready_drives.len()
    );
    println!(
        "machine-cache-oldest-query-ready-age={}",
        oldest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| humantime::format_duration(
                now.duration_since(value)
                    .unwrap_or(std::time::Duration::ZERO)
            )
            .to_string()
            .into()
        )
    );
    println!(
        "machine-cache-newest-query-ready-age={}",
        newest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| humantime::format_duration(
                now.duration_since(value)
                    .unwrap_or(std::time::Duration::ZERO)
            )
            .to_string()
            .into()
        )
    );

    if !verbose {
        return;
    }

    println!("machine-sync-dir={}", daemon_status.sync_dir);
    println!("machine-owner-sid={}", daemon_status.owner_sid);
    println!(
        "machine-cache-oldest-query-ready-at={}",
        oldest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| Cow::from(chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339())
        )
    );
    println!(
        "machine-cache-newest-query-ready-at={}",
        newest_query_ready_at.map_or_else(
            || Cow::from("none"),
            |value| Cow::from(chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339())
        )
    );

    for drive in &daemon_status.published_drives {
        println!("machine-drive-{}-published=true", drive.drive_letter);
        println!(
            "machine-drive-{}-overlay-present={}",
            drive.drive_letter,
            drive.overlay_index_modified_at_unix_ms.is_some()
        );
        println!(
            "machine-drive-{}-snapshot-usn={}",
            drive.drive_letter,
            drive
                .snapshot_usn
                .map_or_else(|| Cow::from("none"), |value| Cow::from(value.to_string()))
        );
        println!(
            "machine-drive-{}-last-usn={}",
            drive.drive_letter,
            drive
                .last_usn
                .map_or_else(|| Cow::from("none"), |value| Cow::from(value.to_string()))
        );
        if let Some(warning) = &drive.warning {
            println!("machine-drive-{}-warning={warning}", drive.drive_letter);
        }
        let query_ready_at = match (
            drive
                .mft_modified_at_unix_ms
                .map(|value| std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)),
            drive
                .base_index_modified_at_unix_ms
                .map(|value| std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)),
        ) {
            (Some(mft_modified_at), Some(index_modified_at)) => {
                Some(mft_modified_at.min(index_modified_at))
            }
            _ => None,
        };
        println!(
            "machine-cache-drive-{}-query-ready={}",
            drive.drive_letter,
            query_ready_at.is_some()
        );
        println!(
            "machine-cache-drive-{}-query-ready-age={}",
            drive.drive_letter,
            query_ready_at.map_or_else(
                || Cow::from("none"),
                |value| humantime::format_duration(
                    now.duration_since(value)
                        .unwrap_or(std::time::Duration::ZERO)
                )
                .to_string()
                .into()
            )
        );
        println!(
            "machine-cache-drive-{}-mft-path={}",
            drive.drive_letter, drive.mft_path
        );
        println!(
            "machine-cache-drive-{}-mft-modified-at={}",
            drive.drive_letter,
            drive.mft_modified_at_unix_ms.map_or_else(
                || Cow::from("none"),
                |value| chrono::DateTime::<chrono::Utc>::from(
                    std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)
                )
                .to_rfc3339()
                .into()
            )
        );
        println!(
            "machine-cache-drive-{}-index-path={}",
            drive.drive_letter, drive.base_index_path
        );
        println!(
            "machine-cache-drive-{}-index-modified-at={}",
            drive.drive_letter,
            drive.base_index_modified_at_unix_ms.map_or_else(
                || Cow::from("none"),
                |value| chrono::DateTime::<chrono::Utc>::from(
                    std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)
                )
                .to_rfc3339()
                .into()
            )
        );
        if verbose {
            println!(
                "machine-drive-{}-mft-path={}",
                drive.drive_letter, drive.mft_path
            );
            println!(
                "machine-drive-{}-mft-modified-at={}",
                drive.drive_letter,
                drive.mft_modified_at_unix_ms.map_or_else(
                    || Cow::from("none"),
                    |value| chrono::DateTime::<chrono::Utc>::from(
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)
                    )
                    .to_rfc3339()
                    .into()
                )
            );
            println!(
                "machine-drive-{}-base-index-path={}",
                drive.drive_letter, drive.base_index_path
            );
            println!(
                "machine-drive-{}-base-index-modified-at={}",
                drive.drive_letter,
                drive.base_index_modified_at_unix_ms.map_or_else(
                    || Cow::from("none"),
                    |value| chrono::DateTime::<chrono::Utc>::from(
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)
                    )
                    .to_rfc3339()
                    .into()
                )
            );
            println!(
                "machine-drive-{}-overlay-index-path={}",
                drive.drive_letter, drive.overlay_index_path
            );
            println!(
                "machine-drive-{}-overlay-index-modified-at={}",
                drive.drive_letter,
                drive.overlay_index_modified_at_unix_ms.map_or_else(
                    || Cow::from("none"),
                    |value| chrono::DateTime::<chrono::Utc>::from(
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)
                    )
                    .to_rfc3339()
                    .into()
                )
            );
            println!(
                "machine-drive-{}-checkpoint-path={}",
                drive.drive_letter, drive.checkpoint_path
            );
            println!(
                "machine-drive-{}-checkpoint-modified-at={}",
                drive.drive_letter,
                drive.checkpoint_modified_at_unix_ms.map_or_else(
                    || Cow::from("none"),
                    |value| chrono::DateTime::<chrono::Utc>::from(
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(value)
                    )
                    .to_rfc3339()
                    .into()
                )
            );
            println!(
                "machine-drive-{}-journal-id={}",
                drive.drive_letter,
                drive
                    .journal_id
                    .map_or_else(|| Cow::from("none"), |value| Cow::from(value.to_string()))
            );
        }
    }
}
