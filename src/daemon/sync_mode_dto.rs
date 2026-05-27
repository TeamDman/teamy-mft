#[derive(Debug, Clone, Copy, PartialEq, Eq, vox::facet::Facet)]
#[repr(u8)]
pub enum SyncModeDto {
    Mft,
    Index,
    Both,
}
