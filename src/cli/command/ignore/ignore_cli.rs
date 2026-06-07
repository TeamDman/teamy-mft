use crate::query::DEFAULT_PROFILE_NAME;
use crate::query::QueryIgnoreRules;
use crate::query::managed_rules_file_name;
use crate::query::normalize_profile_name;
use crate::windows_utils::storage::DriveLetterPattern;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct IgnoreArgs {
    #[facet(args::subcommand)]
    pub command: IgnoreCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum IgnoreCommand {
    /// Append an EXCLUDE rule to the synced rules file in the current sync directory
    Add(IgnoreAddArgs),
    /// List effective `.teamy_mft_rules` files and directives for one profile
    List(IgnoreListArgs),
}

impl IgnoreArgs {
    /// # Errors
    ///
    /// Returns an error if the selected ignore subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            IgnoreCommand::Add(args) => args.invoke(),
            IgnoreCommand::List(args) => args.invoke(),
        }
    }
}

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct IgnoreAddArgs {
    /// Write to the managed rules file for this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Rule pattern to exclude
    #[facet(args::positional)]
    pub pattern: String,
}

impl IgnoreAddArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable or the managed rules file cannot be written.
    pub fn invoke(self) -> eyre::Result<()> {
        let pattern = self.pattern.trim();
        if pattern.is_empty() {
            eyre::bail!("ignore pattern cannot be empty");
        }

        let profile = normalize_profile_name(self.profile.as_deref())?;
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        fs::create_dir_all(&sync_dir)?;
        let rules_path = sync_dir.join(managed_rules_file_name(profile.as_deref()));
        let rendered_rule = format!("EXCLUDE {pattern}");
        let existing = if rules_path.is_file() {
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
        Ok(())
    }
}

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct IgnoreListArgs {
    /// Show effective files for this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Restrict rule discovery to drives matching this pattern
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,
}

impl IgnoreListArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, drive letters cannot be resolved,
    /// or discovered rules files cannot be parsed.
    pub fn invoke(self) -> eyre::Result<()> {
        let profile = normalize_profile_name(self.profile.as_deref())?;
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        let mut seen_paths = BTreeSet::<PathBuf>::new();

        let rules = QueryIgnoreRules::discover_for_drive_letters(
            &drive_letters,
            &sync_dir,
            profile.as_deref(),
        )?;
        for file in rules.files() {
            seen_paths.insert(file.path.clone());
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

        let managed_rules_path = sync_dir.join(managed_rules_file_name(profile.as_deref()));
        if managed_rules_path.is_file() && !seen_paths.contains(&managed_rules_path) {
            if let Some(file) = QueryIgnoreRules::load_rule_file(&managed_rules_path)? {
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
        }

        if rules.files().is_empty() && !managed_rules_path.is_file() {
            println!(
                "No {} files discovered for profile {}.",
                crate::query::RULES_FILE_EXTENSION,
                profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
            );
        }

        Ok(())
    }
}
