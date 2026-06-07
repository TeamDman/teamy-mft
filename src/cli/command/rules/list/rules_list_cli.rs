use crate::query::DEFAULT_PROFILE_NAME;
use crate::query::QueryFilterRules;
use crate::query::normalize_profile_name;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct RulesListArgs {
    /// Show effective files for this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Restrict rule discovery to drives matching this pattern. Compatibility alias: `--drive`.
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
}

impl RulesListArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, drive letters cannot be resolved,
    /// or discovered rules files cannot be parsed.
    pub fn invoke(self) -> eyre::Result<()> {
        let profile = normalize_profile_name(self.profile.as_deref())?;
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;

        let rules = QueryFilterRules::discover_for_drive_letters(
            &drive_letters,
            &sync_dir,
            profile.as_deref(),
        )?;
        for file in rules.files() {
            println!("file={}", file.path.display());
            if let Some(profile_name) = &file.profile {
                println!("profile={profile_name}");
            } else {
                println!("profile={DEFAULT_PROFILE_NAME}");
            }
            for rule in &file.rules {
                println!("  line={}: {}", rule.line_number, rule.render());
            }
        }
        if rules.files().is_empty() {
            println!(
                "No {} files discovered for profile {}.",
                crate::query::RULES_FILE_EXTENSION,
                profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
            );
        }

        Ok(())
    }
}
