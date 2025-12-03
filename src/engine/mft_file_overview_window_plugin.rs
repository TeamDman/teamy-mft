use crate::engine::assets::textures::MyTexture;
use crate::engine::camera_controller_plugin::CameraController;
use crate::engine::camera_controller_plugin::CameraFocusController;
use crate::engine::window_persistence_plugin::PersistWindowProperties;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::WindowIcon;
use bevy::window::WindowRef;

/// Marker component for the overview window entity
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
pub struct MftFileOverviewWindow;

pub struct MftFileOverviewWindowPlugin;

impl Plugin for MftFileOverviewWindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<MftFileOverviewWindow>();
        app.add_systems(Startup, spawn_overview_window_if_missing);
    }
}

const WINDOW_TITLE: &str = "MFT Files - Overview";

fn spawn_overview_window_if_missing(
    mut commands: Commands,
    existing: Query<Entity, With<MftFileOverviewWindow>>,
    asset_server: Res<AssetServer>,
) {
    if existing.is_empty() {
        // Create a new standalone window with the required title
        let window = commands
            .spawn((
                Name::new("MFT File Overview Window"),
                Window {
                    title: WINDOW_TITLE.into(),
                    ..default()
                },
                MftFileOverviewWindow,
                WindowIcon {
                    handle: asset_server.load(MyTexture::Icon),
                },
                PersistWindowProperties::new("preferences/mft_overview_window.ron"),
            ))
            .id();
        debug!(title = WINDOW_TITLE, "Spawned MFT Overview window");

        commands.spawn((
            Name::new("MFT File Overview Window Camera"),
            Camera {
                target: RenderTarget::Window(WindowRef::Entity(window)),
                ..default()
            },
            Camera3d::default(),
            CameraController::default(),
            CameraFocusController::default(),
            // Ensure this camera renders the default world layer (0) and the label layer (1)
            RenderLayers::layer(0).with(1),
            Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }
}
