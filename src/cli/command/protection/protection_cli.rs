use crate::cli::command::protection::disable::ProtectionDisableArgs;
use crate::cli::command::protection::enable::ProtectionEnableArgs;
use crate::cli::command::protection::status::ProtectionStatusArgs;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::path::Path;
use std::path::PathBuf;

#[derive(Facet, Arbitrary, PartialEq, Debug)]
pub struct ProtectionArgs {
    #[facet(args::subcommand)]
    pub command: ProtectionCommand,
}

#[derive(Facet, Arbitrary, PartialEq, Debug)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum ProtectionCommand {
    /// Restore daemon-owned cache ACLs for sensitive MFT artifacts
    Enable(ProtectionEnableArgs),
    /// Temporarily allow broad read access to machine cache artifacts for local development
    Disable(ProtectionDisableArgs),
    /// Show machine cache protection state
    Status(ProtectionStatusArgs),
}

impl Default for ProtectionCommand {
    fn default() -> Self {
        Self::Status(ProtectionStatusArgs)
    }
}

impl ProtectionArgs {
    /// # Errors
    ///
    /// Returns an error if the selected protection subcommand fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.command {
            ProtectionCommand::Enable(args) => args.invoke(),
            ProtectionCommand::Disable(args) => args.invoke(),
            ProtectionCommand::Status(args) => args.invoke(),
        }
    }
}

#[derive(Facet, Arbitrary, PartialEq, Eq, Debug, Clone, Copy, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum ProtectionTarget {
    Mft,
    Index,
    All,
}

impl ProtectionTarget {
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be read.
    pub fn existing_paths(self, sync_dir: &Path) -> eyre::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(sync_dir)? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            match self {
                Self::Mft
                    if file_name.ends_with(crate::machine::config::MFT_CACHE_FILE_EXTENSION) =>
                {
                    paths.push(path);
                }
                Self::Index
                    if file_name.ends_with(crate::machine::config::SEARCH_INDEX_FILE_EXTENSION)
                        || file_name.ends_with(
                            crate::machine::config::OVERLAY_SEARCH_INDEX_FILE_EXTENSION,
                        ) =>
                {
                    paths.push(path);
                }
                Self::All => paths.push(path),
                Self::Mft | Self::Index => {}
            }
        }
        paths.sort();
        Ok(paths)
    }
}
