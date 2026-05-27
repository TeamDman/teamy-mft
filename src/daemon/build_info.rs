#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DaemonBuildInfo {
    pub app_version: String,
    pub git_revision: String,
    pub build_unix_ms: u64,
    pub rpc_compat_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct PingResponse {
    pub service_name: String,
    pub build: DaemonBuildInfo,
}
