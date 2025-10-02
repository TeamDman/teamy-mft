use crate::engine::assets::textures::MyTexture;
use crate::engine::persistence_plugin::Persistable;
use crate::engine::persistence_plugin::PersistenceKey;
use crate::engine::persistence_plugin::PersistenceLoad;
use crate::engine::persistence_plugin::PersistenceLoaded;
use crate::engine::persistence_plugin::PersistencePlugin;
use crate::engine::persistence_plugin::PersistenceProperty;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::window::WindowIcon;
use bevy::window::WindowResolution;

pub struct PrimaryWindowPlugin;

impl Plugin for PrimaryWindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PrimaryWindowMarker>();
        app.add_systems(Startup, setup_primary_window_icon);
        app.add_systems(Startup, setup_primary_window_persistence);
        app.add_systems(Update, handle_window_change);
        app.add_observer(handle_persistence_loaded);
        app.add_plugins(PersistencePlugin::<WindowPersistenceProperty>::default());
    }
}

/// Marker component to identify that this is the primary window entity we're tracking
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
pub struct PrimaryWindowMarker;

#[derive(Debug, Reflect, PartialEq, Clone)]
pub struct WindowPersistenceProperty {
    pub position: WindowPosition,
    pub resolution: WindowResolution,
}

impl Persistable for WindowPersistenceProperty {}

impl From<&Window> for WindowPersistenceProperty {
    fn from(window: &Window) -> Self {
        Self {
            position: window.position,
            resolution: window.resolution.clone(),
        }
    }
}

fn setup_primary_window_icon(
    window: Single<Entity, With<PrimaryWindow>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    commands.entity(*window).insert(WindowIcon {
        handle: asset_server.load(MyTexture::Icon),
    });
    debug!("Primary window icon set");
}

fn setup_primary_window_persistence(
    window: Single<Entity, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    commands
        .entity(*window)
        .insert((
            PrimaryWindowMarker,
            PersistenceKey::<WindowPersistenceProperty>::new("preferences/primary_window.ron"),
            PersistenceLoad::<WindowPersistenceProperty>::default(),
        ));
    debug!("Primary window persistence configured");
}

fn handle_window_change(
    changed: Query<
        (
            Entity,
            &Window,
            Option<&PersistenceProperty<WindowPersistenceProperty>>,
        ),
        (Changed<Window>, With<PrimaryWindowMarker>),
    >,
    mut commands: Commands,
) {
    for (entity, window, persistence) in changed.iter() {
        let new = WindowPersistenceProperty::from(window).into_persistence_property();
        // Avoid change detection if nothing actually changed
        if let Some(old) = persistence
            && *old == new
        {
            continue;
        }

        commands.entity(entity).insert(new);
    }
}

fn handle_persistence_loaded(
    event: On<PersistenceLoaded<WindowPersistenceProperty>>,
    mut windows: Query<&mut Window, With<PrimaryWindowMarker>>,
    mut commands: Commands,
) {
    if let Ok(mut window) = windows.get_mut(event.entity) {
        info!(
            ?event.entity,
            "Applying loaded persistence data to primary window"
        );
        window.position = event.property.position;
        window.resolution = event.property.resolution.clone();

        // Insert the property so it can be tracked for changes
        commands.entity(event.entity).insert(event.property.clone());
    }
}
