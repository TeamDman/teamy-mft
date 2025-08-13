use crate::drive_letter_pattern::DriveLetterPattern;
use arbitrary::Arbitrary;
use clap::Args;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct ListPathsArgs {
    /// Drive letter pattern to match drives whose cached MFTs will be traversed (e.g., "*", "C", "CD", "C,D")
    #[clap(default_value_t = DriveLetterPattern::default())]
    pub drive_pattern: DriveLetterPattern,
}

impl ListPathsArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        // TODO: Read cached .mft files in the sync dir for matching drives and
        // output a newline-delimited list of full file paths to stdout.
        todo!("list-paths not yet implemented");
    }
}

impl crate::cli::to_args::ToArgs for ListPathsArgs {}
