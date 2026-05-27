#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DegradedDriveStatus {
    pub drive_letter: char,
    pub message: String,
}
