use crate::engine::construction::AppConstructionExt;
use bevy::prelude::*;
use tracing::debug;

pub fn run_engine() -> eyre::Result<()> {
    debug!("Building Bevy engine");
    let mut app = App::new_headed()?;

    debug!("Bevy engine built");

    info!("Running Bevy engine");
    app.run();
    Ok(())
}
