use eyre::Context;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::WAIT_FAILED;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::System::Threading::GetExitCodeProcess;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::Owned;

#[derive(Debug)]
pub struct ElevatedChildProcess {
    pub h_process: Owned<HANDLE>,
}

impl ElevatedChildProcess {
    pub fn wait(self) -> eyre::Result<u32> {
        // SAFETY: We own the process handle and keep it open for the entire wait.
        match unsafe { WaitForSingleObject(*self.h_process, INFINITE) } {
            WAIT_OBJECT_0 => {}
            WAIT_FAILED => {
                return Err(windows::core::Error::from_thread())
                    .wrap_err("Failed waiting for elevated process");
            }
            status => eyre::bail!("Unexpected elevated process wait status: {status:?}"),
        }
        let mut code = 0u32;
        // SAFETY: The owned handle is still open and code is a valid output pointer.
        unsafe { GetExitCodeProcess(*self.h_process, &mut code) }
            .wrap_err("Failed to get elevated process exit code")?;
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::ElevatedChildProcess;
    use crate::windows_utils::elevation::ElevationCompleted;
    use windows::Win32::System::Threading::CREATE_EVENT_INITIAL_SET;
    use windows::Win32::System::Threading::CreateEventExW;
    use windows::Win32::System::Threading::EVENT_MODIFY_STATE;
    use windows::core::Owned;

    #[test]
    fn wait_failure_is_an_ordinary_error() {
        // Deliberately omit SYNCHRONIZE access so the wait fails without a UAC prompt.
        // SAFETY: The event is unnamed and uses the default security attributes.
        let handle =
            unsafe { CreateEventExW(None, None, CREATE_EVENT_INITIAL_SET, EVENT_MODIFY_STATE.0) }
                .unwrap();
        let child = ElevatedChildProcess {
            // SAFETY: CreateEventExW returned an owned handle that CloseHandle releases.
            h_process: unsafe { Owned::new(handle) },
        };
        let error = child.wait().unwrap_err();
        assert_eq!(error.to_string(), "Failed waiting for elevated process");
        assert!(error.downcast_ref::<ElevationCompleted>().is_none());
    }
}
