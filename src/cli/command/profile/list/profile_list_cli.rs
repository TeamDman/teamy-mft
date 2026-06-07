use crate::query::QueryFilterRules;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProfileListArgs {
    /// Restrict rule discovery to drives matching this pattern
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
}

impl ProfileListArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, drive letters cannot be resolved,
    /// or discovered rule files cannot be parsed.
    pub fn invoke(self) -> eyre::Result<()> {
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        let files =
            QueryFilterRules::discover_rule_files_for_drive_letters(&drive_letters, &sync_dir)?;

        for profile in QueryFilterRules::discovered_profile_names(&files) {
            println!("{profile}");
        }

        Ok(())
    }
}
