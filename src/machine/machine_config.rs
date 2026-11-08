use crate::machine::config::DEFAULT_IDLE_TIMEOUT_SECS;
use crate::machine::config::DEFAULT_PIPE_NAME;
use crate::machine::config::DEFAULT_SERVICE_NAME;
use crate::machine::config::FacetPathBuf;
use crate::machine::config::default_ignored_rules_path_patterns;
use crate::machine::config::default_sync_dir;
use crate::machine::config::is_access_denied_error;
use crate::machine::config::machine_config_path;
use eyre::ContextCompat;
use facet::Facet;
use std::fs;
use std::io;
use std::path::PathBuf;
use tracing::debug;
use tracing::instrument;

#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct MachineConfig {
    pub version: u32,
    pub owner_sid: String,
    pub sync_dir: FacetPathBuf,
    pub pipe_name: String,
    pub service_name: String,
    pub idle_timeout_secs: u64,
    #[facet(default)]
    pub ignored_rules_path_patterns: Vec<String>,
}

impl MachineConfig {
    #[must_use]
    pub fn new(owner_sid: String, sync_dir: Option<PathBuf>) -> Self {
        Self {
            version: 1,
            owner_sid,
            sync_dir: sync_dir.unwrap_or_else(default_sync_dir).into(),
            pipe_name: String::from(DEFAULT_PIPE_NAME),
            service_name: String::from(DEFAULT_SERVICE_NAME),
            idle_timeout_secs: DEFAULT_IDLE_TIMEOUT_SECS,
            ignored_rules_path_patterns: default_ignored_rules_path_patterns(),
        }
    }

    #[must_use]
    pub fn with_normalized_defaults(mut self) -> Self {
        if self.ignored_rules_path_patterns.is_empty() {
            self.ignored_rules_path_patterns = default_ignored_rules_path_patterns();
        }
        self
    }

    /// Load the installed machine configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the config is missing, unreadable or invalid.
    #[instrument(level = "debug")]
    #[track_caller]
    pub fn load() -> eyre::Result<Self> {
        Self::load_if_present()?
            .wrap_err("Teamy-MFT is not installed. Run `teamy-mft install --help`.")
    }

    /// # Errors
    ///
    /// Returns an error if the machine config cannot be read or parsed.
    pub fn load_if_present() -> eyre::Result<Option<Self>> {
        let path = machine_config_path();
        match fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(metadata) => {
                eyre::bail!(
                    "Machine config path {} exists but is not a file; file_type={:?}",
                    path.display(),
                    metadata.file_type()
                );
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                debug!(path = %path.display(), "Machine config file is not present");
                return Ok(None);
            }
            Err(error) => {
                return Err(eyre::eyre!(
                    "Failed reading machine config metadata at {}: {error}",
                    path.display()
                ));
            }
        }

        let config = facet_json::from_str::<Self>(&fs::read_to_string(&path)?)
            .map_err(|error| eyre::eyre!("Failed parsing {}: {error}", path.display()))?;
        Ok(Some(config.with_normalized_defaults()))
    }

    /// Load client settings, using defaults when no readable config is available.
    ///
    /// # Errors
    ///
    /// Returns an error if the installed config is invalid or cannot be read,
    /// except for permission-denied errors that allow client defaults.
    pub fn load_for_client() -> eyre::Result<Self> {
        match Self::load_if_present() {
            Ok(Some(config)) => Ok(config),
            Ok(None) => Ok(Self::new(String::new(), None)),
            Err(error) if is_access_denied_error(&error) => Ok(Self::new(String::new(), None)),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MachineConfig;
    use crate::machine::config::DEFAULT_IGNORED_RULES_PATH_PATTERNS;
    use crate::machine::config::default_ignored_rules_path_patterns;

    #[test]
    fn new_machine_config_includes_default_ignored_rules_paths() {
        let config = MachineConfig::new(String::from("owner"), None);

        assert_eq!(
            config.ignored_rules_path_patterns,
            default_ignored_rules_path_patterns()
        );
    }

    #[test]
    fn legacy_machine_config_without_ignored_rules_paths_still_parses() {
        let config = facet_json::from_str::<MachineConfig>(
            r#"{
                "version": 1,
                "owner_sid": "owner",
                "sync_dir": "C:\\ProgramData\\teamy_mft\\cache",
                "pipe_name": "\\\\.\\pipe\\teamy-mft-daemon",
                "service_name": "teamy-mft-daemon",
                "idle_timeout_secs": 300
            }"#,
        )
        .expect("legacy machine config should parse")
        .with_normalized_defaults();

        assert_eq!(
            config.ignored_rules_path_patterns,
            default_ignored_rules_path_patterns()
        );
        assert!(
            config
                .ignored_rules_path_patterns
                .iter()
                .any(|pattern| pattern.contains("CARGO_HOME"))
        );
        assert_eq!(
            config.ignored_rules_path_patterns.len(),
            DEFAULT_IGNORED_RULES_PATH_PATTERNS.len()
        );
    }
}
