use crate::machine::config::MachineConfig;
use crate::machine::security::service_sddl;
use crate::windows_utils::string::EasyPCWSTR;
use eyre::WrapErr;
use std::ffi::c_void;
use std::path::Path;
use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;
use windows::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST;
use windows::Win32::Foundation::ERROR_SERVICE_MARKED_FOR_DELETE;
use windows::Win32::Foundation::ERROR_SERVICE_NOT_ACTIVE;
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::Authorization::SDDL_REVISION_1;
use windows::Win32::Security::DACL_SECURITY_INFORMATION;
use windows::Win32::Security::PSECURITY_DESCRIPTOR;
use windows::Win32::System::Services::ChangeServiceConfig2W;
use windows::Win32::System::Services::ControlService;
use windows::Win32::System::Services::CreateServiceW;
use windows::Win32::System::Services::DeleteService;
use windows::Win32::System::Services::OpenSCManagerW;
use windows::Win32::System::Services::OpenServiceW;
use windows::Win32::System::Services::QueryServiceStatusEx;
use windows::Win32::System::Services::SC_MANAGER_CONNECT;
use windows::Win32::System::Services::SC_MANAGER_CREATE_SERVICE;
use windows::Win32::System::Services::SC_STATUS_PROCESS_INFO;
use windows::Win32::System::Services::SERVICE_ALL_ACCESS;
use windows::Win32::System::Services::SERVICE_CONFIG_DESCRIPTION;
use windows::Win32::System::Services::SERVICE_CONTROL_STOP;
use windows::Win32::System::Services::SERVICE_DEMAND_START;
use windows::Win32::System::Services::SERVICE_DESCRIPTIONW;
use windows::Win32::System::Services::SERVICE_ERROR_NORMAL;
use windows::Win32::System::Services::SERVICE_QUERY_STATUS;
use windows::Win32::System::Services::SERVICE_RUNNING;
use windows::Win32::System::Services::SERVICE_START;
use windows::Win32::System::Services::SERVICE_START_PENDING;
use windows::Win32::System::Services::SERVICE_STATUS;
use windows::Win32::System::Services::SERVICE_STATUS_PROCESS;
use windows::Win32::System::Services::SERVICE_STOP;
use windows::Win32::System::Services::SERVICE_STOP_PENDING;
use windows::Win32::System::Services::SERVICE_STOPPED;
use windows::Win32::System::Services::SERVICE_WIN32_OWN_PROCESS;
use windows::Win32::System::Services::SetServiceObjectSecurity;
use windows::Win32::System::Services::StartServiceW;
use windows::core::Error as WindowsError;
use windows::core::HRESULT;
use windows::core::Owned;
use windows::core::PCWSTR;
use windows::core::PWSTR;

#[derive(Debug)]
pub struct ServiceQueryError {
    pub service_name: String,
    pub source: WindowsError,
}

impl ServiceQueryError {
    #[must_use]
    pub fn is_access_denied(&self) -> bool {
        self.source.code() == HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
    }
}

impl std::fmt::Display for ServiceQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Failed querying Windows service {}: {}",
            self.service_name, self.source
        )
    }
}

impl std::error::Error for ServiceQueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsServiceState {
    Missing,
    Stopped,
    StartPending,
    Running,
    Unknown(u32),
}

#[must_use]
pub fn is_development_target_exe(path: &Path) -> bool {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| pair[0] == "target" && (pair[1] == "debug" || pair[1] == "release"))
}

/// # Errors
///
/// Returns an error if the service cannot be registered through the Windows SCM APIs.
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
    // SAFETY: Null machine/database names select the local default SCM database and the returned
    // handle is wrapped in `Owned` for `CloseServiceHandle` cleanup.
    let manager = unsafe {
        Owned::new(
            OpenSCManagerW(
                PCWSTR::null(),
                PCWSTR::null(),
                SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE,
            )
            .wrap_err("Failed opening the Windows service control manager")?,
        )
    };
    let service_name_wide = config.service_name.as_str().easy_pcwstr()?;
    let binary_path_wide = binary_path.as_str().easy_pcwstr()?;
    let local_system = "LocalSystem".easy_pcwstr()?;
    // SAFETY: All wide strings are NUL-terminated and live for the duration of the call, and the
    // returned service handle is wrapped in `Owned` for automatic cleanup.
    let service = unsafe {
        Owned::new(
            CreateServiceW(
                *manager,
                service_name_wide.as_ref(),
                service_name_wide.as_ref(),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_DEMAND_START,
                SERVICE_ERROR_NORMAL,
                binary_path_wide.as_ref(),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                local_system.as_ref(),
                PCWSTR::null(),
            )
            .wrap_err_with(|| {
                format!(
                    "Failed creating Windows service {} through the SCM",
                    config.service_name
                )
            })?,
        )
    };
    let description_text = "Privileged NTFS sync/query daemon for teamy-mft".easy_pcwstr()?;
    let description = SERVICE_DESCRIPTIONW {
        // SAFETY: `description_text` owns a live NUL-terminated buffer for the duration of the
        // API call below, and Windows only borrows this string while updating service metadata.
        lpDescription: unsafe { PWSTR(description_text.as_ptr().0.cast_mut()) },
    };
    // SAFETY: `service` is a valid SCM handle and `description` points at a live, NUL-terminated
    // buffer for the duration of the call.
    unsafe {
        ChangeServiceConfig2W(
            *service,
            SERVICE_CONFIG_DESCRIPTION,
            Some(std::ptr::from_ref(&description).cast::<c_void>()),
        )
        .wrap_err_with(|| {
            format!(
                "Failed setting description for Windows service {}",
                config.service_name
            )
        })?;
    };
    let sddl = service_sddl(&config.owner_sid);
    let sddl = sddl.easy_pcwstr()?;
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: The input SDDL buffer is NUL-terminated and Windows initializes `descriptor` on
    // success. `descriptor` is released via `LocalFreeSecurityDescriptor`.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ref(),
            SDDL_REVISION_1,
            &raw mut descriptor,
            None,
        )
        .wrap_err_with(|| {
            format!(
                "Failed building security descriptor for Windows service {}",
                config.service_name
            )
        })?;
    };
    let descriptor_guard = LocalFreeSecurityDescriptor(descriptor);
    // SAFETY: `service` is a valid SCM handle and `descriptor_guard` owns a valid security
    // descriptor until after the call returns.
    unsafe {
        SetServiceObjectSecurity(*service, DACL_SECURITY_INFORMATION, descriptor_guard.0)
            .wrap_err_with(|| {
                format!(
                    "Failed applying security descriptor to Windows service {}",
                    config.service_name
                )
            })?;
    };
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
    // SAFETY: Null machine/database names select the local default SCM database and the returned
    // handle is wrapped in `Owned` for `CloseServiceHandle` cleanup.
    let manager = unsafe {
        Owned::new(
            OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
                .wrap_err("Failed opening the Windows service control manager")?,
        )
    };
    let service_name_wide = service_name.easy_pcwstr()?;
    // SAFETY: `service_name_wide` is NUL-terminated and lives across the call; the returned
    // handle is wrapped in `Owned` for automatic cleanup.
    let service = unsafe {
        Owned::new(
            OpenServiceW(*manager, service_name_wide.as_ref(), SERVICE_ALL_ACCESS)
                .wrap_err_with(|| format!("Failed opening Windows service {service_name}"))?,
        )
    };
    // SAFETY: `service` is a live handle opened from the SCM above.
    if let Err(error) = unsafe { DeleteService(*service) }
        && error.code() != HRESULT::from_win32(ERROR_SERVICE_MARKED_FOR_DELETE.0)
    {
        return Err(error)
            .wrap_err_with(|| format!("Failed deleting Windows service {service_name}"));
    }
    wait_for_missing(service_name, std::time::Duration::from_secs(10))?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the service state cannot be queried.
pub fn query_service_state(service_name: &str) -> eyre::Result<WindowsServiceState> {
    // SAFETY: Null machine/database names select the local default SCM database and the returned
    // handle is wrapped in `Owned` for `CloseServiceHandle` cleanup.
    let manager = unsafe {
        Owned::new(
            OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
                .wrap_err("Failed opening the Windows service control manager")?,
        )
    };
    let service_name_wide = service_name.easy_pcwstr()?;
    // SAFETY: `service_name_wide` is NUL-terminated and lives across the call.
    let service =
        match unsafe { OpenServiceW(*manager, service_name_wide.as_ref(), SERVICE_QUERY_STATUS) } {
            // SAFETY: The opened handle is valid on success and `Owned` ensures cleanup.
            Ok(service) => unsafe { Owned::new(service) },
            Err(error) if error.code() == HRESULT::from_win32(ERROR_SERVICE_DOES_NOT_EXIST.0) => {
                return Ok(WindowsServiceState::Missing);
            }
            Err(error) => {
                return Err(ServiceQueryError {
                    service_name: service_name.to_owned(),
                    source: error,
                }
                .into());
            }
        };

    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut bytes_needed = 0;
    // SAFETY: `status` is a POD buffer, and we expose its exact byte span for the API to fill in
    // place.
    let status_buffer = unsafe {
        std::slice::from_raw_parts_mut(
            std::ptr::from_mut(&mut status).cast::<u8>(),
            std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
        )
    };
    // SAFETY: `service` is valid, `status_buffer` points at writable storage of the documented
    // size, and `bytes_needed` points at writable storage for the returned byte count.
    if let Err(error) = unsafe {
        QueryServiceStatusEx(
            *service,
            SC_STATUS_PROCESS_INFO,
            Some(status_buffer),
            &raw mut bytes_needed,
        )
    } {
        return Err(ServiceQueryError {
            service_name: service_name.to_owned(),
            source: error,
        }
        .into());
    }

    Ok(match status.dwCurrentState {
        SERVICE_START_PENDING => WindowsServiceState::StartPending,
        SERVICE_RUNNING => WindowsServiceState::Running,
        SERVICE_STOPPED | SERVICE_STOP_PENDING => WindowsServiceState::Stopped,
        other => WindowsServiceState::Unknown(other.0),
    })
}

#[must_use]
pub fn is_service_query_access_denied(error: &eyre::Report) -> bool {
    error
        .chain()
        .filter_map(|source| source.downcast_ref::<ServiceQueryError>())
        .any(ServiceQueryError::is_access_denied)
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
    // SAFETY: Null machine/database names select the local default SCM database and the returned
    // handle is wrapped in `Owned` for `CloseServiceHandle` cleanup.
    let manager = unsafe {
        Owned::new(
            OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
                .wrap_err("Failed opening the Windows service control manager")?,
        )
    };
    let service_name_wide = service_name.easy_pcwstr()?;
    // SAFETY: `service_name_wide` is NUL-terminated and lives across the call; the returned
    // handle is wrapped in `Owned` for automatic cleanup.
    let service = unsafe {
        Owned::new(
            OpenServiceW(*manager, service_name_wide.as_ref(), SERVICE_START)
                .wrap_err_with(|| format!("Failed opening Windows service {service_name}"))?,
        )
    };
    // SAFETY: `service` is a valid SCM handle and we intentionally pass no service arguments.
    if let Err(error) = unsafe { StartServiceW(*service, None) }
        && error.code() != HRESULT::from_win32(ERROR_SERVICE_ALREADY_RUNNING.0)
    {
        return Err(error)
            .wrap_err_with(|| format!("Failed starting Windows service {service_name}"));
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

    // SAFETY: Null machine/database names select the local default SCM database and the returned
    // handle is wrapped in `Owned` for `CloseServiceHandle` cleanup.
    let manager = unsafe {
        Owned::new(
            OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
                .wrap_err("Failed opening the Windows service control manager")?,
        )
    };
    let service_name_wide = service_name.easy_pcwstr()?;
    // SAFETY: `service_name_wide` is NUL-terminated and lives across the call; the returned
    // handle is wrapped in `Owned` for automatic cleanup.
    let service = unsafe {
        Owned::new(
            OpenServiceW(*manager, service_name_wide.as_ref(), SERVICE_STOP)
                .wrap_err_with(|| format!("Failed opening Windows service {service_name}"))?,
        )
    };
    let mut status = SERVICE_STATUS::default();
    // SAFETY: `service` is a valid SCM handle and `status` points at writable storage for the
    // control API to populate.
    if let Err(error) = unsafe { ControlService(*service, SERVICE_CONTROL_STOP, &raw mut status) }
        && error.code() != HRESULT::from_win32(ERROR_SERVICE_NOT_ACTIVE.0)
    {
        return Err(error)
            .wrap_err_with(|| format!("Failed stopping Windows service {service_name}"));
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

struct LocalFreeSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for LocalFreeSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: The descriptor came from `ConvertStringSecurityDescriptorToSecurityDescriptorW`
            // and must be released with `LocalFree`.
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0.cast()))) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServiceQueryError;
    use super::is_development_target_exe;
    use std::path::Path;
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows::Win32::Foundation::ERROR_SERVICE_DOES_NOT_EXIST;
    use windows::core::Error as WindowsError;
    use windows::core::HRESULT;

    #[test]
    fn detects_cargo_debug_and_release_exes() {
        assert!(is_development_target_exe(Path::new(
            r"G:\repo\target\debug\teamy-mft.exe"
        )));
        assert!(is_development_target_exe(Path::new(
            r"G:\repo\target\release\teamy-mft.exe"
        )));
    }

    #[test]
    fn rejects_non_cargo_target_paths() {
        assert!(!is_development_target_exe(Path::new(
            r"C:\Program Files\teamy-mft\teamy-mft.exe"
        )));
        assert!(!is_development_target_exe(Path::new(
            r"G:\repo\target\profile\teamy-mft.exe"
        )));
    }

    #[test]
    fn detects_access_denied_service_query_errors() {
        let error = ServiceQueryError {
            service_name: String::from("teamy-mft-daemon"),
            source: WindowsError::from_hresult(HRESULT::from_win32(ERROR_ACCESS_DENIED.0)),
        };

        assert!(error.is_access_denied());
    }

    #[test]
    fn ignores_non_access_denied_service_query_errors() {
        let error = ServiceQueryError {
            service_name: String::from("teamy-mft-daemon"),
            source: WindowsError::from_hresult(HRESULT::from_win32(ERROR_SERVICE_DOES_NOT_EXIST.0)),
        };

        assert!(!error.is_access_denied());
    }
}
