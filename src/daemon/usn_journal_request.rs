#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct UsnJournalRequest {
    pub drive_letter: char,
}

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct CreateUsnJournalRequest {
    pub drive_letter: char,
    pub maximum_size: u64,
    pub allocation_delta: u64,
}
