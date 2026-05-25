use facet::Facet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum DaemonLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DaemonLogField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DaemonLogEvent {
    pub timestamp_unix_ms: u64,
    pub level: DaemonLogLevel,
    pub target: String,
    pub message: String,
    pub request_id: u64,
    pub method: String,
    pub query_transaction: Option<String>,
    pub fields: Vec<DaemonLogField>,
}

unsafe impl vox_types::Reborrow for DaemonLogEvent {
    type Ref<'a> = DaemonLogEvent;
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct IndexedPathRowDto {
    pub path: String,
    pub has_deleted_entries: bool,
    pub is_ignored: bool,
}

unsafe impl vox_types::Reborrow for IndexedPathRowDto {
    type Ref<'a> = IndexedPathRowDto;
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct QueryRequest {
    pub query: Vec<String>,
    pub query_scope: Option<String>,
    pub drive_letters: Vec<char>,
    pub limit: usize,
    pub include_deleted: bool,
    pub only_deleted: bool,
    pub show_ignored: bool,
    pub only_ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct QueryResponse {
    pub rows: Vec<IndexedPathRowDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SyncRequest {
    pub drive_letters: Vec<char>,
    pub mode: SyncModeDto,
    pub if_exists: IfExistsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet, Default)]
pub struct StatusRequest;

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct LogStreamRequest {
    pub replay_recent: bool,
    pub follow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct StatusResponse {
    pub loaded_drive_letters: Vec<char>,
    pub degraded_drives: Vec<DegradedDriveStatus>,
    pub buffered_log_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DegradedDriveStatus {
    pub drive_letter: char,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DaemonBuildInfo {
    pub app_version: String,
    pub git_revision: String,
    pub rpc_compat_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PingResponse {
    pub service_name: String,
    pub build: DaemonBuildInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum SyncModeDto {
    Mft,
    Index,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum IfExistsDto {
    Skip,
    Overwrite,
    Abort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum MachineErrorKind {
    Unavailable,
    Degraded,
    RequestInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct MachineError {
    pub kind: MachineErrorKind,
    pub message: String,
}

impl std::fmt::Display for MachineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MachineError {}

impl MachineError {
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: MachineErrorKind::Unavailable,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn degraded(message: impl Into<String>) -> Self {
        Self {
            kind: MachineErrorKind::Degraded,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn request_invalid(message: impl Into<String>) -> Self {
        Self {
            kind: MachineErrorKind::RequestInvalid,
            message: message.into(),
        }
    }
}

#[vox::service]
pub trait MachineDaemonRpc {
    async fn ping(&self, logs: vox::Tx<DaemonLogEvent>) -> Result<PingResponse, MachineError>;

    async fn query(
        &self,
        request: QueryRequest,
        logs: vox::Tx<DaemonLogEvent>,
    ) -> Result<QueryResponse, MachineError>;

    async fn query_stream(
        &self,
        request: QueryRequest,
        rows: vox::Tx<IndexedPathRowDto>,
        logs: vox::Tx<DaemonLogEvent>,
    ) -> Result<(), MachineError>;

    async fn sync(
        &self,
        request: SyncRequest,
        logs: vox::Tx<DaemonLogEvent>,
    ) -> Result<(), MachineError>;

    async fn status(
        &self,
        request: StatusRequest,
        logs: vox::Tx<DaemonLogEvent>,
    ) -> Result<StatusResponse, MachineError>;

    async fn stream_logs(
        &self,
        request: LogStreamRequest,
        logs: vox::Tx<DaemonLogEvent>,
        cancel: vox::Rx<u8>,
    ) -> Result<(), MachineError>;
}
