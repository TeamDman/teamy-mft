use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;
use std::path::PathBuf;
use tracing::info;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct SetSyncDirArgs {
    /// Path to set as sync directory (defaults to current working directory if omitted)
    pub path: Option<PathBuf>,
}

impl SetSyncDirArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        let target = if let Some(p) = self.path {
            dunce::canonicalize(p)?
        } else {
            dunce::canonicalize(std::env::current_dir()?)?
        };
        info!("Setting sync dir to {}", target.display());
        crate::sync_dir::set_sync_dir(target.clone())?;
        println!("Set sync dir to {}", target.display());
        Ok(())
    }
}

impl ToArgs for SetSyncDirArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        let mut v = Vec::new();
        if let Some(p) = &self.path {
            v.push(p.clone().into());
        }
        v
    }
}
