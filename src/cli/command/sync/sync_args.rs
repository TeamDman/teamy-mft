use crate::cli::command::sync::sync_index_command::invoke_sync_index;
use crate::cli::command::sync::sync_mft_command::invoke_sync_mft;
use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;
use clap::Subcommand;
use std::ffi::OsString;
use teamy_windows::storage::DriveLetterPattern;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct SyncArgs {
    /// Sync stage to run (omit to run all stages: mft then index)
    #[clap(subcommand)]
    pub mode: Option<SyncMode>,

    /// Drive letter pattern to match drives to sync (e.g., "*", "C", "CD", "C,D")
    #[clap(long, default_value_t = DriveLetterPattern::default())]
    pub drive_pattern: DriveLetterPattern,

    /// Overwrite existing cached MFT files
    #[clap(long, default_value_t = Default::default())]
    pub if_exists: IfExistsOutputBehaviour,
}

#[derive(Subcommand, Arbitrary, PartialEq, Debug, Clone)]
pub enum SyncMode {
    /// Sync raw .mft snapshots
    Mft,
    /// Build .mft_search_index files from snapshots
    Index,
}

#[derive(Default, Arbitrary, clap::ValueEnum, Clone, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum IfExistsOutputBehaviour {
    /// Skip existing files
    Skip,
    /// Overwrite existing files
    #[default]
    Overwrite,
    /// Abort the operation if any existing files are found
    Abort,
}

impl SyncArgs {
    /// Sync MFT data from drives.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync directory cannot be retrieved, elevation fails,
    /// or if reading/writing MFT data fails.
    ///
    /// # Panics
    ///
    /// Panics if spawning worker threads fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match self.mode {
            None => {
                let snapshots = invoke_sync_mft(&self, true)?;
                invoke_sync_index(&self, Some(&snapshots))
            }
            Some(SyncMode::Mft) => {
                invoke_sync_mft(&self, false)?;
                Ok(())
            }
            Some(SyncMode::Index) => invoke_sync_index(&self, None),
        }
    }
}

impl ToArgs for SyncArgs {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if let Some(mode) = &self.mode {
            match mode {
                SyncMode::Mft => args.push("mft".into()),
                SyncMode::Index => args.push("index".into()),
            }
        }
        if self.drive_pattern != DriveLetterPattern::default() {
            args.push("--drive-pattern".into());
            args.push(self.drive_pattern.as_ref().into());
        }
        if self.if_exists != IfExistsOutputBehaviour::default() {
            args.push("--if-exists".into());
            args.push(self.if_exists.to_string().into());
        }
        args
    }
}
