use crate::engine::assets::asset_message_log_plugin::AssetMessageLogPlugin;
use crate::engine::egui_plugin::MyEguiPlugin;
use crate::engine::mft_file_overview_window_plugin::MftFileOverviewWindowPlugin;
use crate::engine::mft_file_plugin::MftFilePlugin;
use crate::engine::sync_dir_plugin::SyncDirectoryPlugin;
use crate::engine::world_inspector_plugin::MyWorldInspectorPlugin;
use bevy::dev_tools::fps_overlay::FpsOverlayConfig;
use bevy::dev_tools::fps_overlay::FpsOverlayPlugin;
use bevy::dev_tools::fps_overlay::FrameTimeGraphConfig;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::text::FontSmoothing;
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
                // primary_window: None,
                exit_condition: ExitCondition::OnAllClosed,
                ..default()
            })
            .disable::<LogPlugin>(), // we initialized tracing already
    );
    app.add_plugins(SyncDirectoryPlugin);
    app.add_plugins(MftFilePlugin);
    app.add_plugins(MftFileOverviewWindowPlugin);
    app.add_plugins(AssetMessageLogPlugin);
    app.add_plugins(MyEguiPlugin);
    app.add_plugins(MyWorldInspectorPlugin);
    app.add_plugins(FpsOverlayPlugin {
        config: FpsOverlayConfig {
            text_config: TextFont {
                // Here we define size of our overlay
                font_size: 42.0,
                // If we want, we can use a custom font
                font: default(),
                // We could also disable font smoothing,
                font_smoothing: FontSmoothing::default(),
                ..default()
            },
            // We can also change color of the overlay
            text_color: Color::srgb(0.0, 1.0, 0.0),
            // We can also set the refresh interval for the FPS counter
            refresh_interval: core::time::Duration::from_millis(100),
            enabled: true,
            frame_time_graph_config: FrameTimeGraphConfig {
                enabled: true,
                // The minimum acceptable fps
                min_fps: 30.0,
                // The target fps
                target_fps: 144.0,
            },
        },
    });
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((Name::new("Primary Window Camera"), Camera2d));
    });

    debug!("Bevy engine built");

    info!("Running Bevy engine");
    app.run();
    Ok(())
}
