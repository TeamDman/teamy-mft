#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct LogStreamRequest {
    pub replay_recent: bool,
    pub follow: bool,
}
