use crate::domain::DriveLetter;

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct DegradedDriveStatus {
    pub drive_letter: DriveLetter,
    pub message: String,
}
