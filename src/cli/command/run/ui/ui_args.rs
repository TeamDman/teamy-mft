use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::engine::construction::AppConstructionExt;
use arbitrary::Arbitrary;
use bevy::prelude::*;
use clap::Args;
use tracing::debug;
#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct UiArgs {}

impl UiArgs {
    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        debug!("Building Bevy engine");
        let mut app = App::new_headed(global_args)?;
        debug!("Bevy engine built");

        info!("Running Bevy engine");
        app.run();
        Ok(())
    }
}

impl ToArgs for UiArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        vec![]
    }
}
