use crate::sync::IfExistsOutputBehaviour;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
pub struct SyncPlan {
    /// Drive letter pattern to match drives to sync (e.g., "*", "C", "CD", "C,D")
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// How to handle existing output files
    #[facet(args::named, default)]
    pub if_exists: IfExistsOutputBehaviour,
}
