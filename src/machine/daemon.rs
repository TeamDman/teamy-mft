use crate::cli::command::sync::{
    IfExistsOutputBehaviour, SyncCommand, resolve_drive_infos_in_dir_for_letters,
};
use crate::machine::config::{
    MachineConfig, PublishedCheckpoint, current_unix_ms, load_checkpoint, load_machine_config,
    published_drive_paths, save_checkpoint,
};
use crate::machine::ipc::{
    IfExistsDto, MachineError, MachineRequest, MachineResponse, PipeSecurityAttributes,
    QueryRequest, QueryResponse, SyncModeDto, SyncRequest, create_server,
};
use crate::machine::live_drive_state::LiveDriveState;
use crate::machine::usn::VolumeUsnJournal;
use crate::search_index::format::SEARCH_INDEX_VERSION;
use rustc_hash::FxHashMap;
use std::ffi::c_void;
use std::sync::atomic::AtomicIsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP,
    SERVICE_CONTROL_INTERROGATE, SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE, SERVICE_STATUS_HANDLE,
    SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
    SetServiceStatus, StartServiceCtrlDispatcherW,
};
use windows::core::PCWSTR;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static SERVICE_STATUS_HANDLE_SLOT: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug)]
struct DaemonRuntimeState {
    cache_root: std::path::PathBuf,
    drives: FxHashMap<char, LiveDriveState>,
    degraded: FxHashMap<char, String>,
}

impl DaemonRuntimeState {
    fn new(config: &MachineConfig) -> Self {
        Self {
            cache_root: config.cache_root.clone(),
            drives: FxHashMap::default(),
            degraded: FxHashMap::default(),
        }
    }

    fn query(
        &mut self,
        request: QueryRequest,
    ) -> Result<Vec<crate::query::IndexedPathRow>, MachineError> {
        let mut rows = Vec::new();
        for &drive in &request.drive_letters {
            self.refresh_drive(drive)?;
            let mut per_drive_request = request.clone();
            per_drive_request.drive_letters = vec![drive];
            per_drive_request.limit = 0;
            rows.extend(self.drive_mut(drive)?.query(&per_drive_request)?);
        }

        if request.limit > 0 && rows.len() > request.limit {
            rows.truncate(request.limit);
        }
        Ok(rows)
    }

    fn sync(&mut self, request: SyncRequest) -> Result<(), MachineError> {
        self.flush_dirty_drives();
        sync_machine_cache(
            &self.cache_root,
            &request.drive_letters,
            request.mode,
            request.if_exists,
        )
        .map_err(|error| MachineError::degraded(error.to_string()))?;

        for &drive in &request.drive_letters {
            self.drives.remove(&drive);
            self.degraded.remove(&drive);
            self.refresh_drive(drive)?;
            self.drive_mut(drive)?
                .flush_published()
                .map_err(|error| MachineError::degraded(error.to_string()))?;
        }

        Ok(())
    }

    fn refresh_loaded_drives(&mut self) {
        let drives = self.drives.keys().copied().collect::<Vec<_>>();
        for drive in drives {
            if let Err(error) = self.refresh_drive(drive) {
                warn!(drive = %drive, error = %error.message, "Drive refresh degraded; falling back to disk until next reload");
            }
        }
    }

    fn flush_dirty_drives(&mut self) {
        for (&drive, state) in &mut self.drives {
            if !state.published_dirty() {
                continue;
            }
            if let Err(error) = state.flush_published() {
                warn!(drive = %drive, error = %error, "Failed flushing live overlay during daemon shutdown/idle");
            }
        }
    }

    fn refresh_drive(&mut self, drive: char) -> Result<(), MachineError> {
        if let Some(message) = self.degraded.get(&drive).cloned() {
            return Err(MachineError::degraded(message));
        }

        if !self.drives.contains_key(&drive) {
            let state = self
                .load_drive_state(drive)
                .map_err(|error| MachineError::degraded(error.to_string()))?;
            self.drives.insert(drive, state);
        }

        let refresh_result = self
            .drives
            .get_mut(&drive)
            .expect("drive should be loaded before refresh")
            .refresh();
        if let Err(error) = refresh_result {
            self.drives.remove(&drive);
            let message = error.to_string();
            self.degraded.insert(drive, message.clone());
            return Err(MachineError::degraded(message));
        }
        Ok(())
    }

    fn drive_mut(&mut self, drive: char) -> Result<&mut LiveDriveState, MachineError> {
        self.drives
            .get_mut(&drive)
            .ok_or_else(|| MachineError::degraded(format!("Drive {drive} is not loaded")))
    }

    fn load_drive_state(&self, drive: char) -> eyre::Result<LiveDriveState> {
        let paths = published_drive_paths(&self.cache_root, drive);
        if !paths.mft_path.is_file() {
            eyre::bail!(
                "Drive {} has no published MFT snapshot at {}",
                drive,
                paths.mft_path.display()
            );
        }
        if !paths.base_index_path.is_file() {
            eyre::bail!(
                "Drive {} has no published base index at {}",
                drive,
                paths.base_index_path.display()
            );
        }
        LiveDriveState::load(&self.cache_root, paths)
    }
}

/// # Errors
///
/// Returns an error if the daemon runtime cannot be started.
pub fn run_daemon(service_mode: bool) -> eyre::Result<()> {
    if service_mode {
        run_windows_service_dispatcher()
    } else {
        let config = load_machine_config()?.ok_or_else(|| {
            eyre::eyre!("Machine config is not installed. Run `teamy-mft install` first.")
        })?;
        run_daemon_runtime(config)
    }
}

fn run_windows_service_dispatcher() -> eyre::Result<()> {
    let config = load_machine_config()?.ok_or_else(|| {
        eyre::eyre!("Machine config is not installed. Run `teamy-mft install` first.")
    })?;
    let mut service_name = crate::machine::security::encode_wide(&config.service_name);
    let mut table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: windows::core::PWSTR(service_name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];

    unsafe { StartServiceCtrlDispatcherW(table.as_mut_ptr()) }?;
    Ok(())
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut windows::core::PWSTR) {
    if let Err(error) = service_main_impl() {
        eprintln!("teamy-mft daemon service failed: {error:?}");
    }
}

fn service_main_impl() -> eyre::Result<()> {
    STOP_REQUESTED.store(false, Ordering::Relaxed);
    let config = load_machine_config()?.ok_or_else(|| {
        eyre::eyre!("Machine config is not installed. Run `teamy-mft install` first.")
    })?;
    let service_name = crate::machine::security::encode_wide(&config.service_name);
    let handle = unsafe {
        RegisterServiceCtrlHandlerExW(
            PCWSTR(service_name.as_ptr()),
            Some(service_control_handler),
            None,
        )
    }?;
    SERVICE_STATUS_HANDLE_SLOT.store(handle.0 as isize, Ordering::Relaxed);
    set_service_status(handle, SERVICE_START_PENDING)?;
    set_service_status(handle, SERVICE_RUNNING)?;
    let run_result = run_daemon_runtime(config);
    let _ = set_service_status(handle, SERVICE_STOPPED);
    SERVICE_STATUS_HANDLE_SLOT.store(0, Ordering::Relaxed);
    run_result
}

unsafe extern "system" fn service_control_handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut std::ffi::c_void,
    _context: *mut std::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            STOP_REQUESTED.store(true, Ordering::Relaxed);
            if let Some(handle) = current_service_status_handle() {
                let _ = set_service_status(handle, SERVICE_STOP_PENDING);
            }
            NO_ERROR.0
        }
        SERVICE_CONTROL_INTERROGATE => NO_ERROR.0,
        _ => NO_ERROR.0,
    }
}

fn current_service_status_handle() -> Option<SERVICE_STATUS_HANDLE> {
    let raw = SERVICE_STATUS_HANDLE_SLOT.load(Ordering::Relaxed);
    (raw != 0).then_some(SERVICE_STATUS_HANDLE(raw as *mut c_void))
}

fn set_service_status(
    handle: SERVICE_STATUS_HANDLE,
    current_state: SERVICE_STATUS_CURRENT_STATE,
) -> eyre::Result<()> {
    let controls = if current_state == SERVICE_START_PENDING {
        0
    } else {
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
    };
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: current_state,
        dwControlsAccepted: controls,
        dwWin32ExitCode: NO_ERROR.0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };
    unsafe { SetServiceStatus(handle, &status) }?;
    Ok(())
}

fn run_daemon_runtime(config: MachineConfig) -> eyre::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let mut state = DaemonRuntimeState::new(&config);
        let mut security_attributes = PipeSecurityAttributes::for_owner(&config.owner_sid)?;
        let mut last_activity = std::time::Instant::now();
        let idle_timeout = Duration::from_secs(config.idle_timeout_secs);
        let mut first_instance = true;
        let mut server = unsafe {
            create_server(
                &config.pipe_name,
                security_attributes.as_mut_ptr(),
                first_instance,
            )?
        };
        first_instance = false;

        loop {
            if STOP_REQUESTED.load(Ordering::Relaxed) {
                break;
            }
            if last_activity.elapsed() >= idle_timeout {
                break;
            }

            tokio::select! {
                connect_result = server.connect() => {
                    connect_result?;
                    let connected = server;
                    server = unsafe {
                        create_server(
                            &config.pipe_name,
                            security_attributes.as_mut_ptr(),
                            first_instance,
                        )?
                    };
                    if let Err(error) = handle_connection(connected, &mut state).await {
                        tracing::warn!(error = %error, "Daemon request failed");
                    }
                    last_activity = std::time::Instant::now();
                }
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    state.refresh_loaded_drives();
                }
            }
        }

        state.flush_dirty_drives();
        Ok(())
    })
}

async fn handle_connection(
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    state: &mut DaemonRuntimeState,
) -> eyre::Result<()> {
    let request_len = server.read_u32_le().await? as usize;
    let mut request_bytes = vec![0u8; request_len];
    server.read_exact(&mut request_bytes).await?;
    let request = serde_json::from_slice::<MachineRequest>(&request_bytes)?;
    let response = match request {
        MachineRequest::Ping => MachineResponse::Pong,
        MachineRequest::Query(request) => match state.query(request) {
            Ok(rows) => MachineResponse::Query(QueryResponse { rows }),
            Err(error) => MachineResponse::Error(error),
        },
        MachineRequest::Sync(request) => match state.sync(request) {
            Ok(()) => MachineResponse::SyncCompleted,
            Err(error) => MachineResponse::Error(error),
        },
    };
    let response_bytes = serde_json::to_vec(&response)?;
    server
        .write_u32_le(
            response_bytes
                .len()
                .try_into()
                .map_err(|_| eyre::eyre!("response too large"))?,
        )
        .await?;
    server.write_all(&response_bytes).await?;
    server.flush().await?;
    let _ = server.disconnect();
    Ok(())
}

/// # Errors
///
/// Returns an error if sync fails or if overlay/checkpoint sidecars cannot be written.
pub fn sync_machine_cache(
    cache_root: &std::path::Path,
    drive_letters: &[char],
    mode: SyncModeDto,
    if_exists: IfExistsDto,
) -> eyre::Result<()> {
    std::fs::create_dir_all(cache_root)?;
    let drive_infos =
        resolve_drive_infos_in_dir_for_letters(cache_root, drive_letters.iter().copied())?;
    let effective_mode = if matches!(mode, SyncModeDto::Mft) {
        info!(
            drives = ?drive_letters,
            "Machine-managed MFT sync upgrades to full sync so published query state stays coherent"
        );
        SyncModeDto::Both
    } else {
        mode
    };
    let snapshot_cursors = if matches!(effective_mode, SyncModeDto::Both) {
        let mut cursors = FxHashMap::default();
        for drive in drive_letters {
            let journal = VolumeUsnJournal::open(*drive)?;
            cursors.insert(*drive, journal.query_cursor()?);
        }
        Some(cursors)
    } else {
        None
    };
    let if_exists = match if_exists {
        IfExistsDto::Skip => IfExistsOutputBehaviour::Skip,
        IfExistsDto::Overwrite => IfExistsOutputBehaviour::Overwrite,
        IfExistsDto::Abort => IfExistsOutputBehaviour::Abort,
    };
    let sync_command = match mode {
        SyncModeDto::Mft => SyncCommand::Both,
        SyncModeDto::Index => SyncCommand::Index(Default::default()),
        SyncModeDto::Both => SyncCommand::Both,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(sync_command.invoke(drive_infos.clone(), &if_exists))?;

    for info in drive_infos {
        let paths = published_drive_paths(cache_root, info.drive_letter);
        if !paths.overlay_index_path.is_file() {
            crate::search_index::search_index_bytes::SearchIndexBytesMut::from_rows(
                crate::search_index::format::SearchIndexHeader::new(info.drive_letter, 0, 0),
                &[],
            )?
            .write_to_path(&paths.overlay_index_path)?;
        }
        match effective_mode {
            SyncModeDto::Index => {
                if load_checkpoint(&paths.checkpoint_path)?.is_none() {
                    let checkpoint = PublishedCheckpoint {
                        published_at_unix_ms: current_unix_ms(),
                        ..PublishedCheckpoint::empty(info.drive_letter, SEARCH_INDEX_VERSION)
                    };
                    save_checkpoint(&paths.checkpoint_path, &checkpoint)?;
                }
            }
            SyncModeDto::Both => {
                let cursor = snapshot_cursors
                    .as_ref()
                    .and_then(|cursors| cursors.get(&info.drive_letter))
                    .ok_or_else(|| {
                        eyre::eyre!("Missing snapshot cursor for drive {}", info.drive_letter)
                    })?;
                let checkpoint = PublishedCheckpoint {
                    drive_letter: info.drive_letter,
                    volume_serial_number: None,
                    journal_id: Some(cursor.journal_id),
                    snapshot_usn: Some(cursor.next_usn),
                    last_usn: Some(cursor.next_usn),
                    published_at_unix_ms: current_unix_ms(),
                    overlay_row_count: 0,
                    base_index_version: SEARCH_INDEX_VERSION,
                };
                save_checkpoint(&paths.checkpoint_path, &checkpoint)?;
            }
            SyncModeDto::Mft => unreachable!("machine sync Mft mode is normalized to Both"),
        }
    }

    Ok(())
}
