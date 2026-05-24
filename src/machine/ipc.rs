use crate::machine::config::MachineConfig;
use crate::machine::security::{encode_wide, named_pipe_sddl, wide_pcwstr};
use crate::query::IndexedPathRow;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, PipeMode, ServerOptions,
};
use windows::Win32::Foundation::HLOCAL;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineRequest {
    Ping,
    Query(QueryRequest),
    Sync(SyncRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResponse {
    pub rows: Vec<IndexedPathRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncRequest {
    pub drive_letters: Vec<char>,
    pub mode: SyncModeDto,
    pub if_exists: IfExistsDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncModeDto {
    Mft,
    Index,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IfExistsDto {
    Skip,
    Overwrite,
    Abort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineResponse {
    Pong,
    Query(QueryResponse),
    SyncCompleted,
    Error(MachineError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineErrorKind {
    Unavailable,
    Degraded,
    RequestInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineError {
    pub kind: MachineErrorKind,
    pub message: String,
}

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

#[derive(Debug)]
pub struct PipeSecurityAttributes {
    attrs: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl PipeSecurityAttributes {
    /// # Errors
    ///
    /// Returns an error if the pipe security descriptor cannot be constructed.
    pub fn for_owner(owner_sid: &str) -> eyre::Result<Self> {
        let sddl = named_pipe_sddl(owner_sid);
        let wide = encode_wide(&sddl);
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide_pcwstr(&wide),
                1,
                &mut descriptor,
                None,
            )
        }?;

        Ok(Self {
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            },
            descriptor,
        })
    }

    #[must_use]
    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        &mut self.attrs as *mut SECURITY_ATTRIBUTES as *mut c_void
    }
}

impl Drop for PipeSecurityAttributes {
    fn drop(&mut self) {
        if !self.descriptor.0.is_null() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.descriptor.0.cast()))) };
        }
    }
}

/// # Errors
///
/// Returns an error if the daemon pipe cannot be reached or the request exchange fails.
pub fn send_request(
    config: &MachineConfig,
    request: &MachineRequest,
) -> eyre::Result<MachineResponse> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let client = open_client(&config.pipe_name).await?;
        send_request_over_pipe(client, request).await
    })
}

async fn open_client(pipe_name: &str) -> eyre::Result<NamedPipeClient> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match ClientOptions::new()
            .pipe_mode(PipeMode::Message)
            .open(pipe_name)
        {
            Ok(client) => return Ok(client),
            Err(error)
                if error.raw_os_error()
                    == Some(windows::Win32::Foundation::ERROR_PIPE_BUSY.0 as i32)
                    && std::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// # Errors
///
/// Returns an error if the request exchange fails.
pub async fn send_request_over_pipe(
    mut client: NamedPipeClient,
    request: &MachineRequest,
) -> eyre::Result<MachineResponse> {
    let request_bytes = serde_json::to_vec(request)?;
    client
        .write_u32_le(
            request_bytes
                .len()
                .try_into()
                .map_err(|_| eyre::eyre!("request too large"))?,
        )
        .await?;
    client.write_all(&request_bytes).await?;
    client.flush().await?;

    let response_len = client.read_u32_le().await? as usize;
    let mut response_bytes = vec![0u8; response_len];
    client.read_exact(&mut response_bytes).await?;
    Ok(serde_json::from_slice(&response_bytes)?)
}

/// # Errors
///
/// Returns an error if a named pipe server cannot be created.
pub unsafe fn create_server(
    pipe_name: &str,
    security_attributes: *mut c_void,
    first_pipe_instance: bool,
) -> eyre::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .pipe_mode(PipeMode::Message)
        .first_pipe_instance(first_pipe_instance)
        .reject_remote_clients(true)
        .max_instances(1);

    Ok(unsafe { options.create_with_security_attributes_raw(pipe_name, security_attributes) }?)
}

#[cfg(test)]
mod tests {
    use super::{MachineRequest, MachineResponse, QueryRequest, QueryResponse};
    use crate::query::IndexedPathRow;

    #[test]
    fn request_roundtrips_through_json() -> eyre::Result<()> {
        let request = MachineRequest::Query(QueryRequest {
            query: vec![String::from("music"), String::from(".flac$")],
            query_scope: Some(String::from(r"C:\library")),
            drive_letters: vec!['C', 'D'],
            limit: 25,
            include_deleted: true,
            only_deleted: false,
            show_ignored: true,
            only_ignored: false,
        });

        let roundtrip = serde_json::from_slice::<MachineRequest>(&serde_json::to_vec(&request)?)?;
        assert_eq!(roundtrip, request);
        Ok(())
    }

    #[test]
    fn response_roundtrips_through_json() -> eyre::Result<()> {
        let response = MachineResponse::Query(QueryResponse {
            rows: vec![IndexedPathRow {
                path: String::from(r"C:\music\track.flac"),
                has_deleted_entries: false,
                is_ignored: false,
            }],
        });

        let roundtrip = serde_json::from_slice::<MachineResponse>(&serde_json::to_vec(&response)?)?;
        assert_eq!(roundtrip, response);
        Ok(())
    }
}
