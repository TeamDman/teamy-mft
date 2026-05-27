use crate::windows_utils::hicon::get_icon_from_current_module;
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::Win32::UI::WindowsAndMessaging::IDI_APPLICATION;

pub fn get_application_icon() -> eyre::Result<HICON> {
    get_icon_from_current_module(IDI_APPLICATION)
}
