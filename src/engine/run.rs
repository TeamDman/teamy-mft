use crate::engine::assets::asset_message_log_plugin::AssetMessageLogPlugin;
use crate::engine::mft_file_overview_window_plugin::MftFileOverviewWindowPlugin;
use crate::engine::mft_file_plugin::MftFilePlugin;
use crate::engine::sync_dir_plugin::SyncDirectoryPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::window::ExitCondition;
use compact_str::CompactString;
use tracing::debug;

#[derive(Component, Reflect)]
pub struct PhysicalDiskLabel(pub CompactString);

pub fn run_engine() -> eyre::Result<()> {
    debug!("Building Bevy engine");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit, // we want to control the exit behavior ourselves
                ..default()
            })
            .disable::<LogPlugin>(), // we initialized tracing already
    );
    app.add_plugins(SyncDirectoryPlugin);
    app.add_plugins(MftFilePlugin);
    app.add_plugins(MftFileOverviewWindowPlugin);
    app.add_plugins(AssetMessageLogPlugin);
    debug!("Bevy engine built");

    info!("Running Bevy engine");
    app.run();
    Ok(())
}
