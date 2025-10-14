use crate::cli::global_args::GlobalArgs;
use crate::engine::assets::asset_message_log_plugin::AssetMessageLogPlugin;
use crate::engine::camera_controller::CameraControllerPlugin;
use crate::engine::cleanup_plugin::CleanupPlugin;
use crate::engine::directory_children_plugin::DirectoryChildrenPlugin;
use crate::engine::egui_plugin::MyEguiPlugin;
use crate::engine::file_contents_plugin::FileContentsPlugin;
use crate::engine::file_contents_refresh_plugin::FileContentsRefreshPlugin;
use crate::engine::file_metadata_plugin::FileMetadataPlugin;
use crate::engine::file_text_plugin::FileTextPlugin;
use crate::engine::fps_window_plugin::FpsWindowConfig;
use crate::engine::fps_window_plugin::FpsWindowIconConfig;
use crate::engine::fps_window_plugin::FpsWindowPlugin;
use crate::engine::fps_window_plugin::FrameTimeGraphConfig;
use crate::engine::mft_file_brick_plugin::MftFileBrickPlugin;
use crate::engine::mft_file_overview_window_plugin::MftFileOverviewWindowPlugin;
use crate::engine::mft_file_plugin::MftFilePlugin;
use crate::engine::pathbuf_holder_plugin::PathBufHolderPlugin;
use crate::engine::primary_window_plugin::PrimaryWindowPlugin;
use crate::engine::quit_button_window_plugin::QuitButtonWindowPlugin;
use crate::engine::sync_dir_brick_plugin::SyncDirBrickPlugin;
use crate::engine::sync_dir_plugin::SyncDirectoryPlugin;
use crate::engine::timeout_plugin::TimeoutPlugin;
use crate::engine::window_persistence_plugin::WindowPersistencePlugin;
use crate::engine::world_inspector_plugin::MyWorldInspectorPlugin;
use crate::DEFAULT_EXTRA_FILTERS;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::text::FontSmoothing;
use bevy::window::ExitCondition;
use bevy::window::WindowResolution;
use bevy_skein::SkeinPlugin;
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
    fn new_headed(global_args: GlobalArgs) -> Result<Self>;
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
        self.add_plugins(PathBufHolderPlugin);
        self.add_plugins(FileContentsPlugin);
        self.add_plugins(FileContentsRefreshPlugin);
        self.add_plugins(FileTextPlugin);
        self.add_plugins(FileMetadataPlugin);
        self.add_plugins(CleanupPlugin);
        self.add_plugins(DirectoryChildrenPlugin);
        self.add_plugins(TimeoutPlugin);
        self.add_plugins(WindowPersistencePlugin);
        self.add_plugins(SkeinPlugin::default());
        self
    }

    fn new_headed(global_args: GlobalArgs) -> Result<Self> {
        {
            debug!("Building headed Bevy engine");
            let mut app = App::new();
            app.add_plugins(
                DefaultPlugins.set(WindowPlugin {
                    // primary_window: None,
                    exit_condition: ExitCondition::OnAllClosed,
                    ..default()
                }).set(LogPlugin {
                    level: global_args.log_level().into(),
                    filter: DEFAULT_EXTRA_FILTERS.to_string(),
                    ..default()
                }),
            );
            app.add_plugins(MeshPickingPlugin);

            app.add_common_plugins();

            app.add_plugins(PrimaryWindowPlugin);
            app.add_plugins(MftFileOverviewWindowPlugin);
            app.add_plugins(CameraControllerPlugin);
            app.add_plugins(MyEguiPlugin);
            app.add_plugins(QuitButtonWindowPlugin);
            app.add_plugins(MyWorldInspectorPlugin);
            app.add_plugins(MftFileBrickPlugin);
            app.add_plugins(SyncDirBrickPlugin);
            app.add_plugins(FpsWindowPlugin {
                config: FpsWindowConfig {
                    text_config: TextFont {
                        font_size: 42.0,
                        font: default(),
                        font_smoothing: FontSmoothing::default(),
                        ..default()
                    },
                    text_color: Color::srgb(0.0, 1.0, 0.0),
                    refresh_interval: core::time::Duration::from_millis(100),
                    enabled: true,
                    frame_time_graph_config: FrameTimeGraphConfig {
                        enabled: true,
                        min_fps: 30.0,
                        target_fps: 144.0,
                    },
                    window_title: "FPS Overlay".to_owned(),
                    window_resolution: WindowResolution::new(640, 360),
                    icon_config: FpsWindowIconConfig::default(),
                },
            });
            app.add_systems(Startup, |mut commands: Commands| {
                commands.spawn((Name::new("Primary Window Camera"), Camera2d));
                commands.insert_resource(AmbientLight {
                    color: Color::WHITE,
                    brightness: 200.0,
                    ..default()
                });
            });

            debug!("Headed Bevy engine built");
            Ok(app)
        }
    }
}
