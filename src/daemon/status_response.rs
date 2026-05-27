use crate::daemon::DegradedDriveStatus;
use crate::daemon::PublishedDriveStatus;

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct StatusResponse {
    pub cache_root: String,
    pub owner_sid: String,
    pub loaded_drive_letters: Vec<char>,
    pub degraded_drives: Vec<DegradedDriveStatus>,
    pub buffered_log_count: usize,
    pub published_drives: Vec<PublishedDriveStatus>,
}
