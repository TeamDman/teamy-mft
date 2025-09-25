use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft_check::check_drives;
use arbitrary::Arbitrary;
use clap::Args;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct CheckArgs {
    /// Drive letter pattern to match drives whose cached MFTs will be checked (e.g., "*", "C", "CD", "C,D")
    #[clap(default_value_t = DriveLetterPattern::default())]
    pub drive_letter_pattern: DriveLetterPattern,
}

impl CheckArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        check_drives(self.drive_letter_pattern)
    }
}

impl crate::cli::to_args::ToArgs for CheckArgs {}
