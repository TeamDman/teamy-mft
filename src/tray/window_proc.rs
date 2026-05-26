use crate::tray::current_tray_icon;
use crate::tray::set_tray_icon;
use eyre::Result;
use eyre::eyre;
use std::io::Write;
use std::sync::OnceLock;
use teamy_windows::console::console_attach;
use teamy_windows::console::console_create;
use teamy_windows::console::console_detach;
use teamy_windows::log::LOG_BUFFER;
use teamy_windows::tray::WM_TASKBAR_CREATED;
use teamy_windows::tray::WM_USER_TRAY_CALLBACK;
use teamy_windows::tray::delete_tray_icon;
use teamy_windows::tray::re_add_tray_icon;
use tracing::error;
use tracing::info;
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::LRESULT;
use windows::Win32::Foundation::POINT;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::System::Console::ATTACH_PARENT_PROCESS;
use windows::Win32::UI::WindowsAndMessaging::AppendMenuW;
use windows::Win32::UI::WindowsAndMessaging::CreatePopupMenu;
use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;
use windows::Win32::UI::WindowsAndMessaging::DestroyMenu;
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
use windows::Win32::UI::WindowsAndMessaging::EnableMenuItem;
use windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
use windows::Win32::UI::WindowsAndMessaging::MF_BYCOMMAND;
use windows::Win32::UI::WindowsAndMessaging::MF_GRAYED;
use windows::Win32::UI::WindowsAndMessaging::MF_SEPARATOR;
use windows::Win32::UI::WindowsAndMessaging::MF_STRING;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;
use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
use windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;
use windows::Win32::UI::WindowsAndMessaging::TPM_LEFTALIGN;
use windows::Win32::UI::WindowsAndMessaging::TPM_RETURNCMD;
use windows::Win32::UI::WindowsAndMessaging::TPM_RIGHTBUTTON;
use windows::Win32::UI::WindowsAndMessaging::TPM_TOPALIGN;
use windows::Win32::UI::WindowsAndMessaging::TrackPopupMenu;
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;
use windows::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU;
use windows::Win32::UI::WindowsAndMessaging::WM_CREATE;
use windows::Win32::UI::WindowsAndMessaging::WM_DESTROY;
use windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDBLCLK;
use windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP;
use windows::core::PCWSTR;
use windows::core::w;

const CMD_SHOW_LOGS: usize = 0x2200;
const CMD_HIDE_LOGS: usize = 0x2201;
const CMD_EXIT_APP: usize = 0x2202;

#[derive(Debug, Clone)]
pub struct TrayWindowConfig {
    pub inherited_console_available: bool,
}

static TRAY_CONFIG: OnceLock<TrayWindowConfig> = OnceLock::new();

pub fn configure(config: TrayWindowConfig) -> Result<()> {
    TRAY_CONFIG
        .set(config)
        .map_err(|set_error| eyre!("Tray window already configured: {set_error:?}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsoleMode {
    Detached,
    Inherited,
    Owned,
}

struct DaemonLogStreamHandle {
    cancel_tx: Option<vox::Tx<u8>>,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl DaemonLogStreamHandle {
    fn stop(mut self) {
        let _ = self.cancel_tx.take();
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

struct TrayWindowState {
    console_mode: ConsoleMode,
    inherited_console_available: bool,
    daemon_log_stream: Option<DaemonLogStreamHandle>,
}

impl TrayWindowState {
    fn new(config: &TrayWindowConfig) -> Self {
        let console_mode = if config.inherited_console_available {
            ConsoleMode::Inherited
        } else {
            ConsoleMode::Detached
        };
        Self {
            console_mode,
            inherited_console_available: config.inherited_console_available,
            daemon_log_stream: None,
        }
    }

    fn can_show_logs(&self) -> bool {
        self.console_mode != ConsoleMode::Owned
    }

    fn can_hide_logs(&self) -> bool {
        self.console_mode == ConsoleMode::Owned
    }

    fn show_logs(&mut self) {
        if !self.can_show_logs() {
            return;
        }
        if self.console_mode == ConsoleMode::Inherited
            && let Err(error) = console_detach()
        {
            error!("Failed to detach console: {error}");
            return;
        }
        if let Err(error) = console_create() {
            error!("Failed to allocate console: {error}");
            return;
        }
        if let Err(error) = Self::replay_local_logs() {
            error!("Failed to replay local logs: {error}");
        }
        self.start_daemon_log_stream();
        self.console_mode = ConsoleMode::Owned;
        info!("Console window allocated for tray logs");
    }

    fn hide_logs(&mut self) {
        if !self.can_hide_logs() {
            return;
        }
        self.stop_daemon_log_stream();
        if let Err(error) = console_detach() {
            error!("Failed to detach console: {error}");
            return;
        }
        if self.inherited_console_available {
            if let Err(error) = console_attach(ATTACH_PARENT_PROCESS) {
                error!("Failed to reattach to parent console: {error}");
                self.console_mode = ConsoleMode::Detached;
            } else {
                self.console_mode = ConsoleMode::Inherited;
            }
        } else {
            self.console_mode = ConsoleMode::Detached;
        }
    }

    fn replay_local_logs() -> Result<()> {
        let mut stdout = std::io::stdout();
        LOG_BUFFER.replay(&mut stdout)?;
        stdout.flush()?;
        Ok(())
    }

    fn start_daemon_log_stream(&mut self) {
        if self.daemon_log_stream.is_some() {
            return;
        }

        let Some(config) = load_config_for_tray_logs() else {
            let _ = writeln!(
                std::io::stdout(),
                "teamy-mft daemon is not installed yet. Run `teamy-mft install` first."
            );
            let _ = std::io::stdout().flush();
            return;
        };

        let (cancel_tx, cancel_rx) = vox::channel::<u8>();
        let join_handle = std::thread::spawn(move || {
            if let Err(error) = crate::machine::ipc::ensure_daemon_ready(&config) {
                tracing::error!(
                    service_name = %config.service_name,
                    error = %error,
                    "Failed to prepare daemon service for tray log stream"
                );
                return;
            }

            let (logs_tx, logs_rx) = vox::channel::<crate::machine::daemon_log::DaemonLogEvent>();
            let drain_thread = crate::machine::daemon_log::spawn_stderr_log_drain(logs_rx);

            let stream_result = crate::machine::ipc::stream_logs(
                &config,
                crate::machine::ipc::LogStreamRequest {
                    replay_recent: true,
                    follow: true,
                },
                logs_tx,
                cancel_rx,
            );
            match stream_result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(error = %error, "Daemon log stream ended with daemon error");
                }
                Err(error) => tracing::error!(error = %error, "Daemon log stream failed"),
            }

            let _ = drain_thread.join();
        });

        self.daemon_log_stream = Some(DaemonLogStreamHandle {
            cancel_tx: Some(cancel_tx),
            join_handle: Some(join_handle),
        });
    }

    fn stop_daemon_log_stream(&mut self) {
        if let Some(handle) = self.daemon_log_stream.take() {
            handle.stop();
        }
    }

    #[allow(
        clippy::undocumented_unsafe_blocks,
        reason = "Win32 menu interactions are localized here and documented inline where helpful"
    )]
    fn show_context_menu(&mut self, hwnd: HWND) {
        // SAFETY: `hwnd` is the live tray window handle receiving this menu request.
        let _ = unsafe { SetForegroundWindow(hwnd) };
        // SAFETY: Creating a popup menu does not outlive this function and the returned handle is destroyed below.
        let menu = match unsafe { CreatePopupMenu() } {
            Ok(menu) => menu,
            Err(error) => {
                error!("Failed to create context menu: {error}");
                return;
            }
        };

        // SAFETY: The menu handle is valid and the string literals are static wide strings.
        let _ = unsafe { AppendMenuW(menu, MF_STRING, CMD_SHOW_LOGS, w!("Show logs")) };
        // SAFETY: The menu handle is valid and the string literals are static wide strings.
        let _ = unsafe { AppendMenuW(menu, MF_STRING, CMD_HIDE_LOGS, w!("Hide logs")) };
        // SAFETY: The menu handle is valid and the separator entry uses a null label by API contract.
        let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
        // SAFETY: The menu handle is valid and the string literal is a static wide string.
        let _ = unsafe { AppendMenuW(menu, MF_STRING, CMD_EXIT_APP, w!("Exit")) };

        if !self.can_show_logs() {
            // SAFETY: The popup menu handle is valid for the duration of this menu configuration.
            let _ = unsafe {
                EnableMenuItem(
                    menu,
                    CMD_SHOW_LOGS.try_into().expect("CMD_SHOW_LOGS fits in u32"),
                    MF_BYCOMMAND | MF_GRAYED,
                )
            };
        }
        if !self.can_hide_logs() {
            // SAFETY: The popup menu handle is valid for the duration of this menu configuration.
            let _ = unsafe {
                EnableMenuItem(
                    menu,
                    CMD_HIDE_LOGS.try_into().expect("CMD_HIDE_LOGS fits in u32"),
                    MF_BYCOMMAND | MF_GRAYED,
                )
            };
        }

        let mut cursor_pos = POINT::default();
        // SAFETY: `cursor_pos` points to writable stack storage for the current cursor position.
        let _ = unsafe { GetCursorPos(&raw mut cursor_pos) };
        #[allow(
            clippy::cast_sign_loss,
            reason = "Windows APIs return signed command ids and coordinates that are consumed as usize here"
        )]
        let selection = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_TOPALIGN | TPM_LEFTALIGN | TPM_RETURNCMD,
                cursor_pos.x,
                cursor_pos.y,
                None,
                hwnd,
                None,
            )
        }
        .0;

        // SAFETY: `menu` was created in this function and has not yet been destroyed.
        let _ = unsafe { DestroyMenu(menu) };

        #[allow(
            clippy::cast_sign_loss,
            reason = "TrackPopupMenu returns a command id promoted from a signed Win32 type"
        )]
        match selection as usize {
            CMD_SHOW_LOGS => self.show_logs(),
            CMD_HIDE_LOGS => self.hide_logs(),
            CMD_EXIT_APP => {
                // SAFETY: Posting WM_CLOSE back to the tray window is the standard way to request orderly shutdown.
                let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            _ => {}
        }
    }
}

fn load_config_for_tray_logs() -> Option<crate::machine::config::MachineConfig> {
    match crate::machine::config::load_machine_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Failed loading machine daemon config: {error}");
            None
        }
    }
}

#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "These helpers store and recover tray state through the Win32 user-data slot"
)]
fn store_state(hwnd: HWND, state: Box<TrayWindowState>) {
    // SAFETY: We store the boxed state pointer in the window user data slot for later retrieval in this module.
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize) };
}

#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "These helpers store and recover tray state through the Win32 user-data slot"
)]
fn with_state(hwnd: HWND, action: impl FnOnce(&mut TrayWindowState)) {
    // SAFETY: We only read the user data slot associated with this tray window.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if ptr == 0 {
        return;
    }
    // SAFETY: The pointer in the user data slot was created by `store_state` and remains valid until `drop_state`.
    let state = unsafe { &mut *(ptr as *mut TrayWindowState) };
    action(state);
}

#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "These helpers store and recover tray state through the Win32 user-data slot"
)]
fn drop_state(hwnd: HWND) {
    // SAFETY: We clear and recover the pointer stored by `store_state` exactly once during teardown.
    let ptr = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
    if ptr != 0 {
        // SAFETY: The pointer came from `Box::into_raw` in `store_state` and is reclaimed exactly once here.
        unsafe { drop(Box::from_raw(ptr as *mut TrayWindowState)) };
    }
}

#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "The Win32 window procedure must use unsafe FFI calls throughout its message dispatch"
)]
pub unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            if let Some(config) = TRAY_CONFIG.get() {
                store_state(hwnd, Box::new(TrayWindowState::new(config)));
                LRESULT(0)
            } else {
                error!("Tray config missing");
                LRESULT(-1)
            }
        }
        WM_USER_TRAY_CALLBACK => {
            #[allow(
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation,
                reason = "Win32 callback payloads use LPARAM-backed integers that must be matched against message constants"
            )]
            match lparam.0 as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    with_state(hwnd, |state| state.show_context_menu(hwnd));
                }
                WM_LBUTTONDBLCLK => with_state(hwnd, TrayWindowState::show_logs),
                _ => {}
            }
            LRESULT(0)
        }
        m if m == *WM_TASKBAR_CREATED => {
            if let Err(error) = re_add_tray_icon() {
                error!("Failed to re-add tray icon: {error}");
            } else if let Some(icon) = current_tray_icon()
                && let Err(error) = set_tray_icon(icon)
            {
                error!("Failed to restore tray icon after taskbar recreation: {error}");
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // SAFETY: `hwnd` is the active tray window and can be destroyed in response to WM_CLOSE.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Err(error) = delete_tray_icon(hwnd) {
                error!("Failed to delete tray icon: {error}");
            }
            with_state(hwnd, TrayWindowState::hide_logs);
            drop_state(hwnd);
            // SAFETY: Posting quit here terminates the message loop after the tray window is destroyed.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            // SAFETY: Unhandled messages are delegated to the default Win32 window procedure.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}
