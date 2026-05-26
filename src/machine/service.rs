use crate::machine::config::MachineConfig;
use crate::machine::security::service_sddl;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsServiceState {
    Missing,
    Stopped,
    StartPending,
    Running,
    Unknown(u32),
}

/// # Errors
///
/// Returns an error if `sc.exe` cannot be launched or the service cannot be registered.
pub fn install_windows_service(current_exe: &Path, config: &MachineConfig) -> eyre::Result<()> {
    if !matches!(
        query_service_state(&config.service_name)?,
        WindowsServiceState::Missing
    ) {
        eyre::bail!(
            "Service {} is already installed. Run `teamy-mft uninstall` first.",
            config.service_name
        );
    }

    let binary_path = format!("\"{}\" service run --service", current_exe.display());
    run_sc_command([
        "create",
        &config.service_name,
        "type=",
        "own",
        "start=",
        "demand",
        "obj=",
        "LocalSystem",
        "binPath=",
        &binary_path,
    ])?;
    run_sc_command([
        "description",
        &config.service_name,
        "Privileged NTFS sync/query daemon for teamy-mft",
    ])?;
    run_sc_command([
        "sdset",
        &config.service_name,
        &service_sddl(&config.owner_sid),
    ])?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the service cannot be stopped or deleted.
pub fn uninstall_windows_service(service_name: &str) -> eyre::Result<()> {
    if matches!(
        query_service_state(service_name)?,
        WindowsServiceState::Missing
    ) {
        return Ok(());
    }

    let _ = stop_service_if_running(service_name);
    run_sc_command(["delete", service_name])?;
    wait_for_missing(service_name, std::time::Duration::from_secs(10))?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the service state cannot be queried.
pub fn query_service_state(service_name: &str) -> eyre::Result<WindowsServiceState> {
    let output = std::process::Command::new("sc.exe")
        .arg("query")
        .arg(service_name)
        .output()?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        let err = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if text.contains("does not exist") || err.contains("does not exist") {
            return Ok(WindowsServiceState::Missing);
        }
        return Ok(WindowsServiceState::Missing);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("STATE") {
            continue;
        }
        let Some((_, rhs)) = trimmed.split_once(':') else {
            continue;
        };
        let code = rhs
            .split_whitespace()
            .find_map(|token| token.parse::<u32>().ok())
            .unwrap_or_default();
        return Ok(match code {
            2 => WindowsServiceState::StartPending,
            4 => WindowsServiceState::Running,
            1 | 3 => WindowsServiceState::Stopped,
            other => WindowsServiceState::Unknown(other),
        });
    }

    Ok(WindowsServiceState::Unknown(0))
}

/// # Errors
///
/// Returns an error if the service cannot be started.
pub fn start_service_if_needed(service_name: &str) -> eyre::Result<()> {
    match query_service_state(service_name)? {
        WindowsServiceState::Running | WindowsServiceState::StartPending => return Ok(()),
        WindowsServiceState::Missing => {
            eyre::bail!("Service {} is not installed", service_name);
        }
        WindowsServiceState::Stopped | WindowsServiceState::Unknown(_) => {}
    }

    tracing::info!(service_name, "Starting daemon service from client");
    let output = std::process::Command::new("sc.exe")
        .arg("start")
        .arg(service_name)
        .output()?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        if !(stdout.contains("already running") || stderr.contains("already running")) {
            eyre::bail!(
                "Failed starting {}: {}{}",
                service_name,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    wait_for_running(service_name, std::time::Duration::from_secs(10))?;
    tracing::info!(service_name, "Daemon service reached running state");
    Ok(())
}

/// # Errors
///
/// Returns an error if the service cannot be stopped.
pub fn stop_service_if_running(service_name: &str) -> eyre::Result<bool> {
    if !matches!(
        query_service_state(service_name)?,
        WindowsServiceState::Running | WindowsServiceState::StartPending
    ) {
        return Ok(false);
    }

    let output = std::process::Command::new("sc.exe")
        .arg("stop")
        .arg(service_name)
        .output()?;
    if !output.status.success() {
        eyre::bail!(
            "Failed stopping {}: {}{}",
            service_name,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    wait_for_stopped(service_name, std::time::Duration::from_secs(10))?;
    Ok(true)
}

fn wait_for_running(service_name: &str, timeout: std::time::Duration) -> eyre::Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        match query_service_state(service_name)? {
            WindowsServiceState::Running => return Ok(()),
            WindowsServiceState::Missing => {
                eyre::bail!("Service {} disappeared while starting", service_name)
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(150)),
        }
    }
    eyre::bail!(
        "Timed out waiting for {} to enter running state",
        service_name
    )
}

/// # Errors
///
/// Returns an error if the service does not reach the stopped state before `timeout`.
pub fn wait_for_stopped(service_name: &str, timeout: std::time::Duration) -> eyre::Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        match query_service_state(service_name)? {
            WindowsServiceState::Stopped | WindowsServiceState::Missing => return Ok(()),
            WindowsServiceState::Running
            | WindowsServiceState::StartPending
            | WindowsServiceState::Unknown(_) => {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }

    eyre::bail!("Timed out waiting for {} to stop", service_name)
}

fn wait_for_missing(service_name: &str, timeout: std::time::Duration) -> eyre::Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if matches!(
            query_service_state(service_name)?,
            WindowsServiceState::Missing
        ) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    eyre::bail!(
        "Timed out waiting for {} to be deleted from the service manager",
        service_name
    )
}

fn run_sc_command(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> eyre::Result<()> {
    let output = std::process::Command::new("sc.exe").args(args).output()?;
    if !output.status.success() {
        eyre::bail!(
            "sc.exe failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
