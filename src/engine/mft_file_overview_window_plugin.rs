use crate::engine::assets::textures::MyTexture;
use crate::engine::camera_controller::CameraController;
use crate::engine::persistence_plugin::Persistable;
use crate::engine::persistence_plugin::PersistenceKey;
use crate::engine::persistence_plugin::PersistenceLoad;
use crate::engine::persistence_plugin::PersistenceLoaded;
use crate::engine::persistence_plugin::PersistencePlugin;
use crate::engine::persistence_plugin::PersistenceProperty;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::WindowIcon;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;

/// Marker component for the overview window entity
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
pub struct MftFileOverviewWindow;

#[derive(Debug, Reflect, PartialEq, Clone)]
pub struct MftOverviewWindowPersistenceProperty {
    pub position: WindowPosition,
    pub resolution: WindowResolution,
}

impl Persistable for MftOverviewWindowPersistenceProperty {}

impl From<&Window> for MftOverviewWindowPersistenceProperty {
    fn from(window: &Window) -> Self {
        Self {
            position: window.position,
            resolution: window.resolution.clone(),
        }
    }
}

pub struct MftFileOverviewWindowPlugin;

impl Plugin for MftFileOverviewWindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<MftFileOverviewWindow>();
        app.add_systems(Startup, spawn_overview_window_if_missing);
        app.add_systems(Update, handle_window_change);
        app.add_observer(handle_persistence_loaded);
        app.add_plugins(PersistencePlugin::<MftOverviewWindowPersistenceProperty>::default());
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
                PersistenceKey::<MftOverviewWindowPersistenceProperty>::new(
                    "preferences/mft_overview_window.ron",
                ),
                PersistenceLoad::<MftOverviewWindowPersistenceProperty>::default(),
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
            // Ensure this camera renders the default world layer (0) and the label layer (1)
            RenderLayers::layer(0).with(1),
            Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }
}

fn handle_window_change(
    changed: Query<
        (
            Entity,
            &Window,
            Option<&PersistenceProperty<MftOverviewWindowPersistenceProperty>>,
        ),
        (Changed<Window>, With<MftFileOverviewWindow>),
    >,
    mut commands: Commands,
) {
    for (entity, window, persistence) in changed.iter() {
        let new = MftOverviewWindowPersistenceProperty::from(window).into_persistence_property();
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
    event: On<PersistenceLoaded<MftOverviewWindowPersistenceProperty>>,
    mut windows: Query<&mut Window, With<MftFileOverviewWindow>>,
    mut commands: Commands,
) {
    if let Ok(mut window) = windows.get_mut(event.entity) {
        info!(
            ?event,
            "Applying loaded persistence data to MFT overview window"
        );
        window.position = event.property.position;
        window.resolution = event.property.resolution.clone();

        // Insert the property so it can be tracked for changes
        commands.entity(event.entity).insert(event.property.clone());
    }
}
