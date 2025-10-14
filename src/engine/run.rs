use crate::{cli::global_args::GlobalArgs, engine::construction::AppConstructionExt};
use bevy::prelude::*;
use tracing::debug;

pub fn run_engine(global_args: GlobalArgs) -> eyre::Result<()> {
    debug!("Building Bevy engine");
    let mut app = App::new_headed(global_args)?;
    debug!("Bevy engine built");

    info!("Running Bevy engine");
    app.run();
    Ok(())
}
