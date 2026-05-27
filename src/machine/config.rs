use crate::paths::EnsureParentDirExists;
use facet::Facet;
use std::fs;
use std::io;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use tracing::debug;
use tracing::instrument;

pub const MACHINE_ROOT_DIR_NAME: &str = "teamy_mft";
pub const MACHINE_CONFIG_FILE_NAME: &str = "machine_config.json";
pub const DEFAULT_SERVICE_NAME: &str = "teamy-mft-daemon";
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\teamy-mft-daemon";
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct MachineConfig {
    pub version: u32,
    pub owner_sid: String,
    pub sync_dir: FacetPathBuf,
    pub pipe_name: String,
    pub service_name: String,
    pub idle_timeout_secs: u64,
}

impl MachineConfig {
    #[must_use]
    pub fn new(owner_sid: String, sync_dir: Option<PathBuf>) -> Self {
        Self {
            version: 1,
            owner_sid,
            sync_dir: sync_dir.unwrap_or_else(default_sync_dir).into(),
            pipe_name: String::from(DEFAULT_PIPE_NAME),
            service_name: String::from(DEFAULT_SERVICE_NAME),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct FacetPathBuf(PathBuf);

impl FacetPathBuf {
    #[must_use]
    pub fn into_inner(self) -> PathBuf {
        self.0
    }
}

impl Deref for FacetPathBuf {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<PathBuf> for FacetPathBuf {
    fn from(value: PathBuf) -> Self {
        Self(value)
    }
}

impl From<FacetPathBuf> for PathBuf {
    fn from(value: FacetPathBuf) -> Self {
        value.0
    }
}

impl AsRef<Path> for FacetPathBuf {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for FacetPathBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

unsafe fn facet_path_buf_proxy_convert_out(
    target_ptr: facet::PtrConst,
    proxy_ptr: facet::PtrUninit,
) -> Result<facet::PtrMut, String> {
    // SAFETY: `target_ptr` points at a valid `FacetPathBuf` and `proxy_ptr` points at
    // facet-managed storage for a `String` proxy with the correct layout for this conversion.
    unsafe {
        let path = target_ptr.get::<FacetPathBuf>();
        let display_string = path.0.display().to_string();
        let roundtrip = PathBuf::from(&display_string);
        if roundtrip != path.0 {
            return Err(format!(
                "Path {} cannot be safely serialized through display() without loss",
                path.0.display()
            ));
        }

        #[allow(
            clippy::cast_ptr_alignment,
            reason = "facet allocates proxy storage with the alignment required by the proxy type"
        )]
        let proxy_mut = proxy_ptr.as_mut_byte_ptr().cast::<String>();
        proxy_mut.write(display_string);
        Ok(facet::PtrMut::new(proxy_mut.cast::<u8>()))
    }
}

unsafe fn facet_path_buf_proxy_convert_in(
    proxy_ptr: facet::PtrConst,
    target_ptr: facet::PtrUninit,
) -> Result<facet::PtrMut, String> {
    // SAFETY: `proxy_ptr` points at a valid `String` proxy and `target_ptr` points at
    // facet-managed storage for a `FacetPathBuf` destination with the correct layout.
    unsafe {
        let display_string = proxy_ptr.read::<String>();
        let roundtrip = PathBuf::from(&display_string);
        let redisplay = roundtrip.display().to_string();
        if redisplay != display_string {
            return Err(format!(
                "Path {display_string} did not round-trip cleanly through display()"
            ));
        }

        #[allow(
            clippy::cast_ptr_alignment,
            reason = "facet allocates target storage with the alignment required by the target type"
        )]
        let target_mut = target_ptr.as_mut_byte_ptr().cast::<FacetPathBuf>();
        target_mut.write(FacetPathBuf(roundtrip));
        Ok(facet::PtrMut::new(target_mut.cast::<u8>()))
    }
}

const FACET_PATH_BUF_PROXY: facet::ProxyDef = facet::ProxyDef {
    shape: <String as Facet>::SHAPE,
    convert_in: facet_path_buf_proxy_convert_in,
    convert_out: facet_path_buf_proxy_convert_out,
};

// SAFETY: `FacetPathBuf` is serialized through an owned `String` proxy that validates
// lossless round-tripping in both directions before constructing or emitting the path.
unsafe impl Facet<'_> for FacetPathBuf {
    const SHAPE: &'static facet::Shape = &const {
        facet::ShapeBuilder::for_sized::<FacetPathBuf>("FacetPathBuf")
            .module_path("teamy_mft::machine::config")
            .ty(facet::Type::User(facet::UserType::Opaque))
            .def(facet::Def::Scalar)
            .proxy(&FACET_PATH_BUF_PROXY)
            .build()
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PublishedCheckpoint {
    pub drive_letter: char,
    pub volume_serial_number: Option<u32>,
    pub journal_id: Option<u64>,
    pub snapshot_usn: Option<u64>,
    pub last_usn: Option<u64>,
    pub published_at_unix_ms: u64,
    pub overlay_row_count: u64,
    pub base_index_version: u16,
}

impl PublishedCheckpoint {
    #[must_use]
    pub fn empty(drive_letter: char, base_index_version: u16) -> Self {
        Self {
            drive_letter,
            volume_serial_number: None,
            journal_id: None,
            snapshot_usn: None,
            last_usn: None,
            published_at_unix_ms: current_unix_ms(),
            overlay_row_count: 0,
            base_index_version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedDrivePaths {
    pub drive_letter: char,
    pub mft_path: PathBuf,
    pub base_index_path: PathBuf,
    pub overlay_index_path: PathBuf,
    pub checkpoint_path: PathBuf,
}

#[must_use]
pub fn program_data_dir() -> PathBuf {
    std::env::var_os("PROGRAMDATA").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
}

#[must_use]
pub fn machine_root_dir() -> PathBuf {
    program_data_dir().join(MACHINE_ROOT_DIR_NAME)
}

#[must_use]
pub fn machine_config_path() -> PathBuf {
    machine_root_dir().join(MACHINE_CONFIG_FILE_NAME)
}

#[must_use]
pub fn default_sync_dir() -> PathBuf {
    machine_root_dir().join("cache")
}

#[must_use]
pub fn published_drive_paths(sync_dir: &Path, drive_letter: char) -> PublishedDrivePaths {
    PublishedDrivePaths {
        drive_letter,
        mft_path: sync_dir.join(format!("{drive_letter}.mft")),
        base_index_path: sync_dir.join(format!("{drive_letter}.mft_search_index")),
        overlay_index_path: sync_dir.join(format!("{drive_letter}.mft_overlay_search_index")),
        checkpoint_path: sync_dir.join(format!("{drive_letter}.mft_checkpoint.json")),
    }
}

/// # Errors
///
/// Returns an error if the machine config cannot be read or parsed.
pub fn load_machine_config() -> eyre::Result<Option<MachineConfig>> {
    let path = machine_config_path();
    if !path.is_file() {
        debug!(path = %path.display(), "Machine config file is not present");
        return Ok(None);
    }

    let config = facet_json::from_str::<MachineConfig>(&fs::read_to_string(&path)?)
        .map_err(|error| eyre::eyre!("Failed parsing {}: {error}", path.display()))?;
    Ok(Some(config))
}

/// # Errors
///
/// Returns an error if the machine config cannot be written.
pub fn save_machine_config(config: &MachineConfig) -> eyre::Result<()> {
    let path = machine_config_path();
    path.ensure_parent_dir_exists()?;
    let parent = path
        .parent()
        .ok_or_else(|| eyre::eyre!("Machine config path {} has no parent", path.display()))?;
    let test_path = parent.join("machine_config.write_test.tmp");
    let bytes = facet_json::to_vec_pretty(config)?;
    debug!(
        path = %path.display(),
        parent = %parent.display(),
        test_path = %test_path.display(),
        "Saving machine config"
    );

    fs::write(&test_path, b"ok").map_err(|error| {
        eyre::eyre!(
            "Failed creating machine config probe file at {} before writing {}: {error}",
            test_path.display(),
            path.display()
        )
    })?;
    let _ = fs::remove_file(&test_path);

    if path.exists() {
        debug!(
            path = %path.display(),
            "Machine config already exists; repairing permissions before overwrite"
        );
        crate::machine::security::restrict_path_to_owner(&path, &config.owner_sid)?;
        fs::remove_file(&path).map_err(|error| {
            eyre::eyre!(
                "Failed removing stale machine config at {} before overwrite: {error}",
                path.display()
            )
        })?;
    }

    fs::write(&path, &bytes).map_err(|error| {
        eyre::eyre!(
            "Failed writing machine config at {} after successful probe in {}: {error}",
            path.display(),
            parent.display()
        )
    })?;
    Ok(())
}

/// # Errors
///
/// Returns an error if the machine config is not installed or cannot be read.
#[instrument(level = "debug")]
pub fn load_required_machine_config() -> eyre::Result<MachineConfig> {
    load_machine_config()?.ok_or_else(|| {
        eyre::eyre!("Machine daemon is not installed. Run `teamy-mft install` first.")
    })
}

/// # Errors
///
/// Returns an error if the installed machine config exists but cannot be parsed.
pub fn load_machine_client_config() -> eyre::Result<MachineConfig> {
    match load_machine_config() {
        Ok(Some(config)) => Ok(config),
        Ok(None) => Ok(MachineConfig {
            version: 1,
            owner_sid: String::new(),
            sync_dir: default_sync_dir().into(),
            pipe_name: String::from(DEFAULT_PIPE_NAME),
            service_name: String::from(DEFAULT_SERVICE_NAME),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
        }),
        Err(error) if is_access_denied_error(&error) => Ok(MachineConfig {
            version: 1,
            owner_sid: String::new(),
            sync_dir: default_sync_dir().into(),
            pipe_name: String::from(DEFAULT_PIPE_NAME),
            service_name: String::from(DEFAULT_SERVICE_NAME),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
        }),
        Err(error) => Err(error),
    }
}

/// # Errors
///
/// Returns an error if the machine cache root is unavailable because install has not been run.
#[instrument(level = "debug")]
pub fn load_sync_dir_from_config() -> eyre::Result<PathBuf> {
    let config = load_required_machine_config()?;
    debug!(sync_dir = %config.sync_dir.display(), "Resolved machine sync directory");
    Ok(config.sync_dir.into_inner())
}

/// # Errors
///
/// Returns an error if the checkpoint file cannot be read or parsed.
pub fn load_checkpoint(path: &Path) -> eyre::Result<Option<PublishedCheckpoint>> {
    if !path.is_file() {
        return Ok(None);
    }
    let checkpoint = facet_json::from_str::<PublishedCheckpoint>(&fs::read_to_string(path)?)
        .map_err(|error| eyre::eyre!("Failed parsing {}: {error}", path.display()))?;
    Ok(Some(checkpoint))
}

/// # Errors
///
/// Returns an error if the checkpoint file cannot be written.
pub fn save_checkpoint(path: &Path, checkpoint: &PublishedCheckpoint) -> eyre::Result<()> {
    path.ensure_parent_dir_exists()?;
    fs::write(path, facet_json::to_vec_pretty(checkpoint)?)?;
    Ok(())
}

#[must_use]
pub fn current_unix_ms() -> u64 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Unix milliseconds fit in u64 for practical system lifetimes"
    )]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[must_use]
pub fn is_access_denied_error(error: &eyre::Report) -> bool {
    error
        .chain()
        .filter_map(|source| source.downcast_ref::<io::Error>())
        .any(|source| source.kind() == io::ErrorKind::PermissionDenied)
}
