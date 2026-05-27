#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
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
