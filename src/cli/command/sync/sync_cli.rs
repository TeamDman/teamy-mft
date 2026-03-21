use crate::cli::command::sync::sync_index_command::invoke_sync_index;
use crate::cli::command::sync::sync_mft_command::invoke_sync_mft;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use teamy_windows::storage::DriveLetterPattern;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default)]
#[facet(rename_all = "kebab-case")]
pub struct SyncArgs {
    /// Sync stage to run (omit to run all stages: mft then index)
    #[facet(args::subcommand)]
    pub mode: Option<SyncMode>,

    /// Drive letter pattern to match drives to sync (e.g., "*", "C", "CD", "C,D")
    #[facet(args::named, default)]
    pub drive_letter_pattern: DriveLetterPattern,

    /// Overwrite existing cached MFT files
    #[facet(args::named, default)]
    pub if_exists: IfExistsOutputBehaviour,
}

#[derive(Facet, Arbitrary, PartialEq, Debug, Clone)]
#[repr(u8)]
pub enum SyncMode {
    /// Sync raw .mft snapshots
    Mft,
    /// Build `.mft_search_index` files from snapshots
    Index,
}

#[derive(Default, Facet, Arbitrary, Clone, Debug, Eq, PartialEq, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        runtime.block_on(async move {
            match self.mode {
                // None => {
                //     let snapshots = invoke_sync_mft(&self, true).await?;
                //     invoke_sync_index(&self, Some(&snapshots))
                // }
                Some(SyncMode::Mft) => {
                    invoke_sync_mft(&self).await?;
                    Ok(())
                }
                Some(SyncMode::Index) => invoke_sync_index(&self, None),
                _ => todo!(),
            }
        })
    }
}
