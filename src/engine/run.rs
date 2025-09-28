use crate::engine::mft_file_plugin::MftFilePlugin;
use crate::engine::sync_dir_plugin::SyncDirectoryPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::window::ExitCondition;
use compact_str::CompactString;

#[derive(Component, Reflect)]
pub struct PhysicalDiskLabel(pub CompactString);

pub fn run_engine() -> eyre::Result<()> {
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
    app.run();
    Ok(())
}