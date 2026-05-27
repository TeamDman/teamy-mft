#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet, Default)]
pub struct StatusRequest {
    pub drive_letters: Vec<char>,
}
