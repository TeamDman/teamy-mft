use crate::engine::assets::textures::MyTexture;
use crate::engine::window_persistence_plugin::PersistWindowProperties;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::window::WindowIcon;

pub struct PrimaryWindowPlugin;

impl Plugin for PrimaryWindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PrimaryWindowMarker>();
        app.add_systems(Startup, setup_primary_window_icon);
        app.add_systems(Startup, setup_primary_window_persistence);
    }
}

/// Marker component to identify that this is the primary window entity we're tracking
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
pub struct PrimaryWindowMarker;

fn setup_primary_window_icon(
    window: Single<Entity, With<PrimaryWindow>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    commands.entity(*window).insert((
        Name::new("Primary Window"),
        WindowIcon {
            handle: asset_server.load(MyTexture::Icon),
        },
    ));
    debug!("Primary window icon set");
}

fn setup_primary_window_persistence(
    window: Single<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    commands.entity(*window).insert((
        PrimaryWindowMarker,
        PersistWindowProperties::new("preferences/primary_window.ron"),
    ));
    debug!("Primary window persistence configured");
}
