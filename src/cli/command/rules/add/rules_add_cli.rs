use crate::cli::command::rules::RulesMutationDirective;
use crate::paths::EnsureParentDirExists;
use crate::query::QueryIgnoreRules;
use crate::query::RULES_FILE_EXTENSION;
use crate::query::normalize_profile_name;
use crate::query::profile_name_from_rules_path;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use chrono::Local;
use facet::Facet;
use figue::{self as args};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct RulesAddArgs {
    /// Restrict idempotency checks to this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Write to this exact rules file instead of creating a new file in the current working directory
    #[facet(args::named, default)]
    pub rules_file: Option<String>,
    /// Restrict discovered-file idempotency checks to drives matching this pattern
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
    /// Rule sort order for INCLUDE or EXCLUDE directives
    #[facet(args::named, default)]
    pub order: Option<i64>,
    /// Rule directive to append
    #[facet(args::positional)]
    pub directive: RulesMutationDirective,
    /// Rule pattern for INCLUDE or EXCLUDE directives
    #[facet(args::positional, default)]
    pub pattern: String,
}

impl RulesAddArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, rule discovery fails, or the
    /// selected rules file cannot be written.
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

        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        let mut discovered =
            QueryIgnoreRules::discover_rule_files_for_drive_letters(&drive_letters, &sync_dir)?;
        let current_dir = std::env::current_dir()?;
        for file in QueryIgnoreRules::load_rule_files_in_directory(&current_dir)? {
            if discovered.iter().any(|known| known.path == file.path) {
                continue;
            }
            discovered.push(file);
        }
        discovered.sort_by(|left, right| left.path.cmp(&right.path));
        let matching_files = discovered
            .iter()
            .filter(|file| match profile.as_deref() {
                None => file.profile.is_none(),
                Some(profile_name) => {
                    file.profile.is_none() || file.profile.as_deref() == Some(profile_name)
                }
            })
            .filter(|file| file.rules.iter().any(|rule| rule.render() == rendered_rule))
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        if !matching_files.is_empty() {
            println!(
                "Rule already present in {} discovered file(s):",
                matching_files.len()
            );
            for path in matching_files {
                println!("{}", path.display());
            }
            return Ok(());
        }

        let rules_path = if let Some(rules_file) = self.rules_file.as_deref() {
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
                    path_profile.as_deref().unwrap_or("default"),
                    profile.as_deref().unwrap_or("default")
                );
            }
            rules_path
        } else {
            let current_dir = std::env::current_dir()?;
            let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
            let mut attempt = 0usize;
            loop {
                let candidate_name = if let Some(profile_name) = profile.as_deref() {
                    if attempt == 0 {
                        format!("teamy-mft-rules-{timestamp}.{profile_name}{RULES_FILE_EXTENSION}")
                    } else {
                        format!(
                            "teamy-mft-rules-{timestamp}-{attempt}.{profile_name}{RULES_FILE_EXTENSION}"
                        )
                    }
                } else if attempt == 0 {
                    format!("teamy-mft-rules-{timestamp}{RULES_FILE_EXTENSION}")
                } else {
                    format!("teamy-mft-rules-{timestamp}-{attempt}{RULES_FILE_EXTENSION}")
                };
                let candidate = current_dir.join(candidate_name);
                if !candidate.exists() {
                    break candidate;
                }
                attempt += 1;
            }
        };

        rules_path.ensure_parent_dir_exists()?;
        let file_already_existed = rules_path.is_file();
        let existing = if file_already_existed {
            fs::read_to_string(&rules_path)?
        } else {
            String::new()
        };
        if existing.lines().any(|line| line.trim() == rendered_rule) {
            println!("Rule already present in {}", rules_path.display());
            return Ok(());
        }

        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&rules_path)?;
        let mut writer = std::io::BufWriter::new(file);
        writeln!(writer, "{rendered_rule}")?;
        writer.flush()?;
        println!("Added rule to {}", rules_path.display());
        if !file_already_existed
            && !discovered
                .iter()
                .any(|file| file.path.as_path() == rules_path.as_path())
        {
            println!(
                "Run `teamy-mft sync` before querying so the new cwd rules file becomes discoverable from the indexed rule-file path list."
            );
        }
        Ok(())
    }
}
