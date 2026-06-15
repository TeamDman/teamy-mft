use crate::cli::command::rules::RulesMutationDirective;
use crate::query::DEFAULT_PROFILE_NAME;
use crate::query::QueryFilterRules;
use crate::query::RULES_FILE_EXTENSION;
use crate::query::normalize_profile_name;
use crate::query::profile_name_from_rules_path;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::path::PathBuf;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct RulesRemoveArgs {
    /// Restrict removals to this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Remove from this exact rules file instead of all discovered matching files
    #[facet(args::named, default)]
    pub rules_file: Option<String>,
    /// Restrict discovered-file removals to drives matching this pattern
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Rule sort order for INCLUDE or EXCLUDE directives
    #[facet(args::named, default)]
    pub order: Option<i64>,
    /// Rule directive to remove
    #[facet(args::positional)]
    pub directive: RulesMutationDirective,
    /// Rule pattern for INCLUDE or EXCLUDE directives
    #[facet(args::positional, default)]
    pub pattern: String,
}

impl RulesRemoveArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, rule discovery fails, or
    /// matching rule files cannot be rewritten.
    #[expect(
        clippy::too_many_lines,
        reason = "Rule mutation stays procedural here to keep the file-selection flow direct"
    )]
    pub fn invoke(self) -> eyre::Result<()> {
        let profile = normalize_profile_name(self.profile.as_deref())?;
        let pattern = self.pattern.trim();
        let rendered_rule = match self.directive {
            RulesMutationDirective::Include => {
                if pattern.is_empty() {
                    eyre::bail!("include rule pattern cannot be empty");
                }
                if let Some(order) = self.order {
                    format!("ORDER {order} INCLUDE {pattern}")
                } else {
                    format!("INCLUDE {pattern}")
                }
            }
            RulesMutationDirective::Exclude => {
                if pattern.is_empty() {
                    eyre::bail!("exclude rule pattern cannot be empty");
                }
                if let Some(order) = self.order {
                    format!("ORDER {order} EXCLUDE {pattern}")
                } else {
                    format!("EXCLUDE {pattern}")
                }
            }
            RulesMutationDirective::DefaultInclude => {
                if !pattern.is_empty() {
                    eyre::bail!("default-include does not accept a pattern");
                }
                if self.order.is_some() {
                    eyre::bail!("default-include does not accept --order");
                }
                String::from("DEFAULT RULE IS INCLUDE")
            }
            RulesMutationDirective::DefaultExclude => {
                if !pattern.is_empty() {
                    eyre::bail!("default-exclude does not accept a pattern");
                }
                if self.order.is_some() {
                    eyre::bail!("default-exclude does not accept --order");
                }
                String::from("DEFAULT RULE IS EXCLUDE")
            }
        };

        let selected_paths = if let Some(rules_file) = self.rules_file.as_deref() {
            let trimmed = rules_file.trim();
            if trimmed.is_empty() {
                eyre::bail!("--rules-file must not be empty");
            }
            let rules_path = PathBuf::from(trimmed);
            if !rules_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(RULES_FILE_EXTENSION))
            {
                eyre::bail!(
                    "Rules file {} must end with {}",
                    rules_path.display(),
                    RULES_FILE_EXTENSION
                );
            }
            let path_profile = profile_name_from_rules_path(&rules_path)?;
            if path_profile != profile {
                eyre::bail!(
                    "Rules file {} selects profile {:?}, but the command selected profile {:?}",
                    rules_path.display(),
                    path_profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME),
                    profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
                );
            }
            vec![rules_path]
        } else {
            let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
            let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
            let mut discovered =
                QueryFilterRules::discover_rule_files_for_drive_letters(&drive_letters, &sync_dir)?;
            let current_dir = std::env::current_dir()?;
            for file in QueryFilterRules::load_rule_files_in_directory(&current_dir)? {
                if discovered.iter().any(|known| known.path == file.path) {
                    continue;
                }
                discovered.push(file);
            }
            discovered
                .into_iter()
                .filter(|file| match profile.as_deref() {
                    None => file.profile.is_none(),
                    Some(profile_name) => {
                        file.profile.is_none() || file.profile.as_deref() == Some(profile_name)
                    }
                })
                .filter(|file| file.rules.iter().any(|rule| rule.render() == rendered_rule))
                .map(|file| file.path)
                .collect::<Vec<_>>()
        };

        if selected_paths.is_empty() {
            println!(
                "No discovered {} files contain rule `{}` for profile {}.",
                RULES_FILE_EXTENSION,
                rendered_rule,
                profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
            );
            return Ok(());
        }

        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let mut removed_any = false;
        for rules_path in selected_paths {
            if !rules_path.is_file() {
                continue;
            }
            let contents = std::fs::read_to_string(&rules_path)?;
            let kept_lines = contents
                .lines()
                .filter(|line| line.trim() != rendered_rule)
                .collect::<Vec<_>>();
            if kept_lines.len() == contents.lines().count() {
                continue;
            }
            let rewritten = if kept_lines.is_empty() {
                String::new()
            } else {
                format!("{}\n", kept_lines.join("\n"))
            };

            if rewritten.trim().is_empty() {
                std::fs::remove_file(&rules_path)?;
                println!(
                    "Removed `{}` from {} and deleted the now-empty rules file",
                    rendered_rule,
                    rules_path.display()
                );
                let rendered_rules_path = rules_path.to_string_lossy().into_owned();
                match crate::sync::sync_path_into_published_overlay(&sync_dir, &rendered_rules_path)
                {
                    Ok(drive_letter) => {
                        println!(
                            "Ran `teamy-mft sync {}` automatically and updated the published overlay for drive {}.",
                            rules_path.display(),
                            drive_letter
                        );
                    }
                    Err(error) => {
                        println!(
                            "Tried running `teamy-mft sync {}` automatically, but it failed: {}",
                            rules_path.display(),
                            error
                        );
                    }
                }
            } else {
                std::fs::write(&rules_path, rewritten)?;
                println!("Removed `{}` from {}", rendered_rule, rules_path.display());
            }
            removed_any = true;
        }

        if !removed_any {
            println!(
                "Rule `{}` was already absent for profile {}.",
                rendered_rule,
                profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
            );
        }

        Ok(())
    }
}
