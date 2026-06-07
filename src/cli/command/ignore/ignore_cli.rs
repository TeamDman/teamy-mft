use crate::query::QueryIgnoreRules;
use crate::query::SYNCED_IGNORE_FILE_NAME;
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
    /// Append a rule to the synced ignore file in the current sync directory
    Add(IgnoreAddArgs),
    /// List active ignore files and rules discovered from cached indexes
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

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct IgnoreAddArgs {
    /// Rule pattern to append
    #[facet(args::positional)]
    pub pattern: String,
}

impl IgnoreAddArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable or the ignore file cannot be written.
    pub fn invoke(self) -> eyre::Result<()> {
        let pattern = self.pattern.trim();
        if pattern.is_empty() {
            eyre::bail!("ignore pattern cannot be empty");
        }

        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        fs::create_dir_all(&sync_dir)?;
        let ignore_path = sync_dir.join(SYNCED_IGNORE_FILE_NAME);
        let existing = if ignore_path.is_file() {
            fs::read_to_string(&ignore_path)?
        } else {
            String::new()
        };
        if existing.lines().any(|line| line.trim() == pattern) {
            println!("Ignore rule already present in {}", ignore_path.display());
            return Ok(());
        }

        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ignore_path)?;
        let mut writer = std::io::BufWriter::new(file);
        writeln!(writer, "{pattern}")?;
        writer.flush()?;
        println!("Added ignore rule to {}", ignore_path.display());
        Ok(())
    }
}

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct IgnoreListArgs {
    /// Restrict ignore discovery to drives matching this pattern. Compatibility alias: `--drive`.
    #[facet(args::named, args::long_alias = "drive", default)]
    pub drive_letter_pattern: DriveLetterPattern,
}

impl IgnoreListArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, drive letters cannot be resolved,
    /// or discovered ignore files cannot be parsed.
    pub fn invoke(self) -> eyre::Result<()> {
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let drive_letters = self.drive_letter_pattern.into_drive_letters()?;
        let mut seen_paths = BTreeSet::<PathBuf>::new();

        let rules = QueryIgnoreRules::discover_for_drive_letters(&drive_letters, &sync_dir)?;
        for file in rules.files() {
            seen_paths.insert(file.path.clone());
            println!("file={}", file.path.display());
            for rule in &file.rules {
                println!("  line={}: {}", rule.line_number, rule.pattern);
            }
        }

        let synced_ignore_path = sync_dir.join(SYNCED_IGNORE_FILE_NAME);
        if synced_ignore_path.is_file() && !seen_paths.contains(&synced_ignore_path) {
            println!("file={}", synced_ignore_path.display());
            for (index, line) in fs::read_to_string(&synced_ignore_path)?.lines().enumerate() {
                let pattern = line.trim();
                if pattern.is_empty() || pattern.starts_with('#') {
                    continue;
                }
                println!("  line={}: {}", index + 1, pattern);
            }
        }

        if rules.files().is_empty() && !synced_ignore_path.is_file() {
            println!("No ignore files discovered.");
        }

        Ok(())
    }
}
