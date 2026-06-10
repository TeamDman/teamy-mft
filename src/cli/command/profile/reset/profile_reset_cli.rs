use crate::query::DEFAULT_PROFILE_NAME;
use crate::query::QueryFilterRules;
use crate::query::RULES_FILE_EXTENSION;
use crate::query::normalize_profile_name;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use chrono::Local;
use facet::Facet;
use figue::{self as args};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProfileResetArgs {
    /// Profile name to disable; omit for the logical default profile
    #[facet(args::positional, default)]
    pub profile: String,
    /// Restrict rule discovery to drives matching this pattern
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Rename matching files without prompting for confirmation
    #[facet(args::named, default)]
    pub yes: bool,
}

impl ProfileResetArgs {
    /// # Errors
    ///
    /// Returns an error if rule discovery fails, no matching files are found, the user declines
    /// confirmation, or any rename operation fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let requested_profile = normalize_profile_name(
            (!self.profile.trim().is_empty()).then_some(self.profile.as_str()),
        )?;
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        let files =
            QueryFilterRules::discover_rule_files_for_drive_letters(&drive_letters, &sync_dir)?;
        let selected_paths = files
            .into_iter()
            .filter(|file| match requested_profile.as_deref() {
                None => file.profile.is_none(),
                Some(profile) => file.profile.is_none() || file.profile.as_deref() == Some(profile),
            })
            .map(|file| file.path)
            .collect::<BTreeSet<_>>();

        if selected_paths.is_empty() {
            eyre::bail!(
                "No discovered {} files apply to profile {}",
                RULES_FILE_EXTENSION,
                requested_profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
            );
        }

        let profile_name = requested_profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME);
        if !self.yes {
            println!("The following files will be disabled for profile {profile_name}:");
            for path in &selected_paths {
                println!("{}", path.display());
            }
            print!(
                "Rename {} discovered rule file(s) for profile {}? [y/N] ",
                selected_paths.len(),
                profile_name
            );
            std::io::stdout().flush()?;
            let mut response = String::new();
            std::io::stdin().read_line(&mut response)?;
            let confirmed = matches!(response.trim(), "y" | "Y" | "yes" | "YES" | "Yes");
            if !confirmed {
                eyre::bail!("Aborted disabling rule files for profile {}", profile_name);
            }
        }

        let date_suffix = Local::now().format("%Y-%m-%d").to_string();
        for path in selected_paths {
            let target = renamed_disabled_path(&path, &date_suffix)?;
            std::fs::rename(&path, &target).map_err(|error| {
                eyre::eyre!(
                    "Failed renaming {} to {}: {}",
                    path.display(),
                    target.display(),
                    error
                )
            })?;
            println!("disabled={} -> {}", path.display(), target.display());
        }

        Ok(())
    }
}

fn renamed_disabled_path(path: &std::path::Path, date_suffix: &str) -> eyre::Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| eyre::eyre!("{}: invalid UTF-8 filename", path.display()))?;
    Ok(path.with_file_name(format!("{file_name}.{date_suffix}.disabled")))
}
