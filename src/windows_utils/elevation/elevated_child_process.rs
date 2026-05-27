use eyre::eyre;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::GetExitCodeProcess;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::System::Threading::WaitForSingleObject;

#[derive(Debug)]
pub struct ElevatedChildProcess {
    pub h_process: HANDLE,
}

impl ElevatedChildProcess {
    pub fn wait(self) -> eyre::Result<u32> {
        unsafe { WaitForSingleObject(self.h_process, INFINITE) };
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.h_process, &mut code) }
            .map_err(|e| eyre!("Failed to get exit code: {}", e))?;
        unsafe { CloseHandle(self.h_process) }?;
        Ok(code)
    }
}
