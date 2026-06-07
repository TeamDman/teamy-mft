use crate::query::QueryIgnoreRules;
use crate::query::managed_rules_file_name;
use crate::query::normalize_profile_name;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct RulesRemoveArgs {
    /// Remove from the managed rules file for this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Managed rules file line number to remove
    #[facet(args::positional)]
    pub line: usize,
}

impl RulesRemoveArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable, the managed rules file cannot be
    /// parsed, or the selected line cannot be removed.
    pub fn invoke(self) -> eyre::Result<()> {
        let profile = normalize_profile_name(self.profile.as_deref())?;
        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let rules_path = sync_dir.join(managed_rules_file_name(profile.as_deref()));
        if !rules_path.is_file() {
            eyre::bail!("No managed rules file exists at {}", rules_path.display());
        }

        let Some(file) = QueryIgnoreRules::load_rule_file(&rules_path)? else {
            eyre::bail!("No managed rules file exists at {}", rules_path.display());
        };
        let Some(rule) = file.rules.iter().find(|rule| rule.line_number == self.line) else {
            eyre::bail!(
                "No managed rule exists at {}:{}",
                rules_path.display(),
                self.line
            );
        };

        let contents = std::fs::read_to_string(&rules_path)?;
        let kept_lines = contents
            .lines()
            .enumerate()
            .filter_map(|(index, line)| (index + 1 != self.line).then_some(line))
            .collect::<Vec<_>>();
        let rewritten = if kept_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", kept_lines.join("\n"))
        };

        if rewritten.trim().is_empty() {
            std::fs::remove_file(&rules_path)?;
            println!(
                "Removed line {} from {} and deleted the now-empty managed rules file",
                self.line,
                rules_path.display()
            );
        } else {
            std::fs::write(&rules_path, rewritten)?;
            println!(
                "Removed line {} from {}: {}",
                self.line,
                rules_path.display(),
                rule.render()
            );
        }

        Ok(())
    }
}
