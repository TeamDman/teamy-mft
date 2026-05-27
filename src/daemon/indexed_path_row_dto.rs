#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct IndexedPathRowDto {
    pub path: String,
    pub has_deleted_entries: bool,
    pub is_ignored: bool,
}

unsafe impl vox_types::Reborrow for IndexedPathRowDto {
    type Ref<'a> = IndexedPathRowDto;
}
