use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Facet, Arbitrary, PartialEq, Debug, Clone, Copy, Default, Eq)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum SyncMode {
    /// Sync raw .mft snapshots
    Mft,
    /// Build `.mft_search_index` files from snapshots
    Index,
    /// Sync both stages sequentially, with preflight checks and error handling for both stages
    #[default]
    Both,
}
