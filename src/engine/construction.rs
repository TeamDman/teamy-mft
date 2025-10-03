use crate::engine::assets::asset_message_log_plugin::AssetMessageLogPlugin;
use crate::engine::cleanup_plugin::CleanupPlugin;
use crate::engine::directory_children_plugin::DirectoryChildrenPlugin;
use crate::engine::egui_plugin::MyEguiPlugin;
use crate::engine::file_bytes_plugin::FileBytesPlugin;
use crate::engine::mft_file_brick_plugin::MftFileBrickPlugin;
use crate::engine::mft_file_overview_window_plugin::MftFileOverviewWindowPlugin;
use crate::engine::mft_file_plugin::MftFilePlugin;
use crate::engine::pathbuf_holder_plugin::PathBufHolderPlugin;
use crate::engine::predicate::predicate::PredicatePlugin;
use crate::engine::predicate::predicate_file_extension::FileExtensionPredicatePlugin;
use crate::engine::predicate::predicate_path_exists::PathExistsPredicatePlugin;
use crate::engine::predicate::predicate_string_ends_with::StringEndsWithPredicatePlugin;
use crate::engine::primary_window_plugin::PrimaryWindowPlugin;
use crate::engine::sync_dir_plugin::SyncDirectoryPlugin;
use crate::engine::timeout_plugin::TimeoutPlugin;
use crate::engine::world_inspector_plugin::MyWorldInspectorPlugin;
use crate::engine::bytes_plugin::BytesPlugin;
use bevy::dev_tools::fps_overlay::FpsOverlayConfig;
use bevy::dev_tools::fps_overlay::FpsOverlayPlugin;
use bevy::dev_tools::fps_overlay::FrameTimeGraphConfig;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::text::FontSmoothing;
use bevy::window::ExitCondition;
use eyre::Result;
use tracing::debug;

#[derive(Resource, Debug, Clone, Reflect, Default)]
#[reflect(Resource)]
pub struct Headless;

#[derive(Resource, Debug, Clone, Reflect, Default)]
#[reflect(Resource)]
pub struct Testing;

pub trait AppConstructionExt
where
    Self: Sized,
{
    /// Construct the engine with all the systems that make sense when no rendering is needed.
    /// This also inserts the [`Headless`] resource.
    fn new_headless() -> Result<Self>;
    /// Add the common plugins used by both headed and headless engines.
    fn add_common_plugins(&mut self) -> &mut Self;
    /// Construct the engine with all the systems that make sense when rendering is needed.
    fn new_headed() -> Result<Self>;
}
impl AppConstructionExt for App {
    fn new_headless() -> Result<Self> {
        {
            debug!("Building headless Bevy engine");
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            app.add_common_plugins();
            app.init_resource::<Headless>();
            debug!("Headless Bevy engine built");
            Ok(app)
        }
    }

    fn add_common_plugins(&mut self) -> &mut Self {
        self.add_plugins(SyncDirectoryPlugin);
        self.add_plugins(MftFilePlugin);
        self.add_plugins(AssetMessageLogPlugin);
        self.add_plugins(BytesPlugin);
        self.add_plugins(PathBufHolderPlugin);
        self.add_plugins(FileBytesPlugin);
        self.add_plugins(CleanupPlugin);
        self.add_plugins(DirectoryChildrenPlugin);
        self.add_plugins(TimeoutPlugin);
        self.add_plugins(PredicatePlugin);
        self.add_plugins(FileExtensionPredicatePlugin);
        self.add_plugins(StringEndsWithPredicatePlugin);
        self.add_plugins(PathExistsPredicatePlugin);
        self
    }

    fn new_headed() -> Result<Self> {
        {
            debug!("Building headed Bevy engine");
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

            app.add_common_plugins();

            app.add_plugins(PrimaryWindowPlugin);
            app.add_plugins(MftFileOverviewWindowPlugin);
            app.add_plugins(MyEguiPlugin);
            app.add_plugins(MyWorldInspectorPlugin);
            app.add_plugins(MftFileBrickPlugin);
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

            debug!("Headed Bevy engine built");
            Ok(app)
        }
    }
}
