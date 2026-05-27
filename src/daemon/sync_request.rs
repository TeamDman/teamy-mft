use crate::daemon::IfExistsDto;
use crate::daemon::SyncModeDto;

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct SyncRequest {
    pub drive_letters: Vec<char>,
    pub mode: SyncModeDto,
    pub if_exists: IfExistsDto,
}
