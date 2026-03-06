use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use tracing::info;

#[derive(Facet, PartialEq, Debug, Default)]
pub struct SetSyncDirArgs {
    /// Path to set as sync directory (defaults to current working directory if omitted)
    #[facet(args::positional)]
    pub path: Option<String>,
}

impl<'a> Arbitrary<'a> for SetSyncDirArgs {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut path = Option::<String>::arbitrary(u)?;
        if let Some(value) = &mut path
            && value.starts_with('-')
        {
            value.insert(0, 'p');
        }
        Ok(Self { path })
    }
}

impl SetSyncDirArgs {
    /// Set the sync directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be canonicalized or set.
    pub fn invoke(self) -> eyre::Result<()> {
        let target = if let Some(p) = self.path {
            dunce::canonicalize(p)?
        } else {
            dunce::canonicalize(std::env::current_dir()?)?
        };
        info!("Setting sync dir to {}", target.display());
        crate::sync_dir::set_sync_dir(&target)?;
        println!("Set sync dir to {}", target.display());
        Ok(())
    }
}
