use crate::daemon::CorrelationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, vox::facet::Facet, strum::Display)]
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

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DaemonLogField {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DaemonLogSpan {
    pub name: String,
    pub target: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DaemonLogEvent {
    pub timestamp_unix_ms: u64,
    pub level: DaemonLogLevel,
    pub target: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub request_id: u64,
    pub method: String,
    pub correlation_id: Option<CorrelationId>,
    pub spans: Vec<DaemonLogSpan>,
    pub fields: Vec<DaemonLogField>,
}

unsafe impl vox_types::Reborrow for DaemonLogEvent {
    type Ref<'a> = DaemonLogEvent;
}

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DaemonLogWireEvent {
    pub timestamp_unix_ms: u64,
    pub level: DaemonLogLevel,
    pub target: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub request_id: u64,
    pub method: String,
    pub correlation_id: Option<String>,
    pub spans: Vec<DaemonLogSpan>,
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
            file: value.file.clone(),
            line: value.line,
            message: value.message.clone(),
            request_id: value.request_id,
            method: value.method.clone(),
            correlation_id: value.correlation_id.as_ref().map(ToString::to_string),
            spans: value.spans.clone(),
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
            file: value.file,
            line: value.line,
            message: value.message,
            request_id: value.request_id,
            method: value.method,
            correlation_id: value
                .correlation_id
                .map(|correlation_id| correlation_id.parse())
                .transpose()?,
            spans: value.spans,
            fields: value.fields,
        })
    }
}
