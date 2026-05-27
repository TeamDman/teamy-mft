use crate::windows_utils::module::get_current_module;
use eyre::bail;
use tracing::debug;
use tracing::instrument;
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::Win32::UI::WindowsAndMessaging::LoadIconW;
use windows::core::PCWSTR;
use windows::core::Param;

#[instrument]
pub fn get_icon_from_current_module(
    icon_name: impl Param<PCWSTR> + std::fmt::Debug,
) -> eyre::Result<HICON> {
    let handle = get_current_module()?;
    debug!(?handle, "Trying to load embedded icon from current module");
    let icon = unsafe { LoadIconW(Some(HINSTANCE(handle.0)), icon_name) };
    match icon {
        Ok(icon) => {
            debug!("Successfully loaded embedded icon from current module");
            Ok(icon)
        }
        Err(e) => bail!("Failed to load embedded icon from current module: {}", e),
    }
}
