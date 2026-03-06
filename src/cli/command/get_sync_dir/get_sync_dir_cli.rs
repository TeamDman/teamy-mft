use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
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
