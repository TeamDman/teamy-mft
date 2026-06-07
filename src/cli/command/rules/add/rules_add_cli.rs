use crate::query::managed_rules_file_name;
use crate::query::normalize_profile_name;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::fs;
use std::io::Write;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct RulesAddArgs {
    /// Write to the managed rules file for this profile; omit for the logical default profile
    #[facet(args::named, default)]
    pub profile: Option<String>,
    /// Rule sort order for INCLUDE or EXCLUDE directives
    #[facet(args::named, default)]
    pub order: Option<i64>,
    /// Rule directive to append
    #[facet(args::positional)]
    pub directive: RulesAddDirective,
    /// Rule pattern for INCLUDE or EXCLUDE directives
    #[facet(args::positional, default)]
    pub pattern: String,
}

#[derive(Facet, Arbitrary, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum RulesAddDirective {
    #[default]
    Include,
    Exclude,
    DefaultInclude,
    DefaultExclude,
}

impl RulesAddArgs {
    /// # Errors
    ///
    /// Returns an error if the machine cache is unavailable or the managed rules file cannot be written.
    pub fn invoke(self) -> eyre::Result<()> {
        let profile = normalize_profile_name(self.profile.as_deref())?;
        let pattern = self.pattern.trim();
        let rendered_rule = match self.directive {
            RulesAddDirective::Include => {
                if pattern.is_empty() {
                    eyre::bail!("include rule pattern cannot be empty");
                }
                if let Some(order) = self.order {
                    format!("ORDER {order} INCLUDE {pattern}")
                } else {
                    format!("INCLUDE {pattern}")
                }
            }
            RulesAddDirective::Exclude => {
                if pattern.is_empty() {
                    eyre::bail!("exclude rule pattern cannot be empty");
                }
                if let Some(order) = self.order {
                    format!("ORDER {order} EXCLUDE {pattern}")
                } else {
                    format!("EXCLUDE {pattern}")
                }
            }
            RulesAddDirective::DefaultInclude => {
                if !pattern.is_empty() {
                    eyre::bail!("default-include does not accept a pattern");
                }
                if self.order.is_some() {
                    eyre::bail!("default-include does not accept --order");
                }
                String::from("DEFAULT RULE IS INCLUDE")
            }
            RulesAddDirective::DefaultExclude => {
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
        fs::create_dir_all(&sync_dir)?;
        let rules_path = sync_dir.join(managed_rules_file_name(profile.as_deref()));
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
