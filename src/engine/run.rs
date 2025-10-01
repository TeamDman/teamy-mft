use crate::engine::construction::AppConstructionExt;
use crate::engine::sync_dir_plugin::begin_load_sync_dir_from_preferences;
use bevy::prelude::*;
use tracing::debug;

pub fn run_engine() -> eyre::Result<()> {
    debug!("Building Bevy engine");
    let mut app = App::new_headed()?;
    app.add_systems(Startup, begin_load_sync_dir_from_preferences);

    debug!("Bevy engine built");

    info!("Running Bevy engine");
    app.run();
    Ok(())
}
