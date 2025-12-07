use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct GetSyncDirArgs;

impl GetSyncDirArgs {
    /// Get the sync directory.
    ///
    /// # Errors
    ///
    /// Returns an error if retrieving the sync directory fails.
    pub fn invoke(self) -> eyre::Result<()> {
        match crate::sync_dir::get_sync_dir()? {
            Some(p) => println!("{}", p.display()),
            None => println!("<not set>"),
        }
        Ok(())
    }
}

impl ToArgs for GetSyncDirArgs {}
