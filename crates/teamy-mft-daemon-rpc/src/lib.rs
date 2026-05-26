use facet::Facet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Facet)]
#[repr(transparent)]
pub struct CorrelationId(pub Uuid);

impl CorrelationId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for CorrelationId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for CorrelationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

unsafe impl vox_types::Reborrow for CorrelationId {
    type Ref<'a> = CorrelationId;
}

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
    pub correlation_id: Option<CorrelationId>,
    pub fields: Vec<DaemonLogField>,
}

unsafe impl vox_types::Reborrow for DaemonLogEvent {
    type Ref<'a> = DaemonLogEvent;
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct DaemonLogWireEvent {
    pub timestamp_unix_ms: u64,
    pub level: DaemonLogLevel,
    pub target: String,
    pub message: String,
    pub request_id: u64,
    pub method: String,
    pub correlation_id: Option<String>,
    pub fields: Vec<DaemonLogField>,
}

unsafe impl vox_types::Reborrow for DaemonLogWireEvent {
    type Ref<'a> = DaemonLogWireEvent;
}

impl From<&DaemonLogEvent> for DaemonLogWireEvent {
    fn from(value: &DaemonLogEvent) -> Self {
        Self {
            timestamp_unix_ms: value.timestamp_unix_ms,
            level: value.level,
            target: value.target.clone(),
            message: value.message.clone(),
            request_id: value.request_id,
            method: value.method.clone(),
            correlation_id: value.correlation_id.as_ref().map(ToString::to_string),
            fields: value.fields.clone(),
        }
    }
}

impl TryFrom<DaemonLogWireEvent> for DaemonLogEvent {
    type Error = uuid::Error;

    fn try_from(value: DaemonLogWireEvent) -> Result<Self, Self::Error> {
        Ok(Self {
            timestamp_unix_ms: value.timestamp_unix_ms,
            level: value.level,
            target: value.target,
            message: value.message,
            request_id: value.request_id,
            method: value.method,
            correlation_id: value
                .correlation_id
                .map(|correlation_id| correlation_id.parse())
                .transpose()?,
            fields: value.fields,
        })
    }
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
    pub correlation_id: CorrelationId,
    pub rows: Vec<IndexedPathRowDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct QueryStreamResponse {
    pub correlation_id: CorrelationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct SyncRequest {
    pub drive_letters: Vec<char>,
    pub mode: SyncModeDto,
    pub if_exists: IfExistsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet, Default)]
pub struct StatusRequest {
    pub drive_letters: Vec<char>,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct LogStreamRequest {
    pub replay_recent: bool,
    pub follow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct StatusResponse {
    pub cache_root: String,
    pub owner_sid: String,
    pub loaded_drive_letters: Vec<char>,
    pub degraded_drives: Vec<DegradedDriveStatus>,
    pub buffered_log_count: usize,
    pub published_drives: Vec<PublishedDriveStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PublishedDriveStatus {
    pub drive_letter: char,
    pub mft_path: String,
    pub mft_modified_at_unix_ms: Option<u64>,
    pub base_index_path: String,
    pub base_index_modified_at_unix_ms: Option<u64>,
    pub overlay_index_path: String,
    pub overlay_index_modified_at_unix_ms: Option<u64>,
    pub checkpoint_path: String,
    pub checkpoint_modified_at_unix_ms: Option<u64>,
    pub snapshot_usn: Option<u64>,
    pub last_usn: Option<u64>,
    pub journal_id: Option<u64>,
    pub warning: Option<String>,
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
    pub build_unix_ms: u64,
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
    async fn ping(&self, logs: vox::Tx<DaemonLogWireEvent>) -> Result<PingResponse, MachineError>;

    async fn shutdown(&self, logs: vox::Tx<DaemonLogWireEvent>) -> Result<(), MachineError>;

    async fn query(
        &self,
        request: QueryRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<QueryResponse, MachineError>;

    async fn query_stream(
        &self,
        request: QueryRequest,
        rows: vox::Tx<IndexedPathRowDto>,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<QueryStreamResponse, MachineError>;

    async fn sync(
        &self,
        request: SyncRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<(), MachineError>;

    async fn status(
        &self,
        request: StatusRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
    ) -> Result<StatusResponse, MachineError>;

    async fn stream_logs(
        &self,
        request: LogStreamRequest,
        logs: vox::Tx<DaemonLogWireEvent>,
        cancel: vox::Rx<u8>,
    ) -> Result<(), MachineError>;
}
