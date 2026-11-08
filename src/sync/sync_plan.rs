use crate::domain::ParallelismLimit;
use crate::sync::IfExistsOutputBehaviour;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, PartialEq, Debug, Arbitrary, Clone, Default)]
pub struct SyncPlan {
    /// Maximum concurrent index builds and shared indexing CPU workers (0: Rayon automatic). Drive reads remain parallel.
    #[facet(args::named, default)]
    pub parallel: ParallelismLimit,

    /// Drive letter pattern to match drives to sync (e.g., "*", "C", "CD", "C,D"). Compatibility alias: `--drive`.
    #[facet(args::named, args::alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// How to handle existing output files
    #[facet(args::named, default)]
    pub if_exists: IfExistsOutputBehaviour,

    /// When syncing a path, recurse through a directory subtree and refresh overlay rows for all descendants.
    #[facet(args::named, default)]
    pub recursive: bool,

    /// Optional path to reflect into the published overlay index without rebuilding a full drive index
    #[facet(args::positional, default)]
    pub path: Option<String>,
}
