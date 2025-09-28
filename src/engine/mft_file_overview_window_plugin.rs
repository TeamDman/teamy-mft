use crate::engine::assets::textures::MyTexture;
use crate::engine::window_icon_plugin::WindowIcon;
use bevy::prelude::*;

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
        commands.spawn((
            Window {
                title: WINDOW_TITLE.into(),
                ..default()
            },
            MftFileOverviewWindow,
            WindowIcon::new(asset_server.load(MyTexture::Icon)),
        ));
        info!(title = WINDOW_TITLE, "Spawned MFT overview window");
    }
}
