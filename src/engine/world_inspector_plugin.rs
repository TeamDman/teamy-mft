use crate::engine::assets::textures::MyTexture;
use crate::engine::persistence_plugin::Persistable;
use crate::engine::persistence_plugin::PersistenceKey;
use crate::engine::persistence_plugin::PersistenceLoad;
use crate::engine::persistence_plugin::PersistenceLoaded;
use crate::engine::persistence_plugin::PersistencePlugin;
use crate::engine::persistence_plugin::PersistenceProperty;
use bevy::app::MainSchedulePlugin;
use bevy::camera::RenderTarget;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::window::WindowIcon;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;
use bevy_inspector_egui::DefaultInspectorConfigPlugin;
use bevy_inspector_egui::bevy_egui::EguiContext;
use bevy_inspector_egui::bevy_egui::EguiMultipassSchedule;
use bevy_inspector_egui::bevy_egui::EguiPlugin;
use bevy_inspector_egui::bevy_inspector;
use bevy_inspector_egui::egui;
use bevy_inspector_egui::egui::gui_zoom::zoom_menu_buttons;
use itertools::Itertools;

#[derive(Event, Debug, Clone)]
pub enum WorldInspectorWindowEvent {
    SpawnWindow,
    DespawnWindow,
    ToggleWindow,
}
const DEFAULT_SIZE: UVec2 = UVec2::new(500, 500);

pub struct MyWorldInspectorPlugin;

#[derive(Debug, Component, Reflect)]
pub struct WorldInspectorWindow;

#[derive(Debug, Component, Reflect)]
pub struct WorldInspectorWindowCamera;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorldInspectorWindowEguiContextPass;

#[derive(Debug, Reflect, PartialEq, Clone)]
pub struct WorldInspectorWindowPersistenceProperty {
    pub position: WindowPosition,
    pub resolution: WindowResolution,
}
impl Persistable for WorldInspectorWindowPersistenceProperty {}
impl From<&Window> for WorldInspectorWindowPersistenceProperty {
    fn from(window: &Window) -> Self {
        Self {
            position: window.position,
            resolution: window.resolution.clone(),
        }
    }
}

impl Plugin for MyWorldInspectorPlugin {
    fn build(&self, app: &mut App) {
        check_plugins(app, "WorldInspectorPlugin");
        app.add_plugins(DefaultInspectorConfigPlugin);
        app.add_systems(WorldInspectorWindowEguiContextPass, ui);
        app.add_observer(handle_spawn_window_event);
        app.add_observer(handle_despawn_window_event);
        app.add_observer(handle_toggle_window_event);
        app.add_observer(handle_persistence_loaded);
        app.add_systems(Startup, |mut commands: Commands| {
            // Open the window
            commands.trigger(WorldInspectorWindowEvent::SpawnWindow);
        });
        app.add_systems(Update, handle_window_change);
        app.add_plugins(PersistencePlugin::<WorldInspectorWindowPersistenceProperty>::default());
    }
}

fn handle_spawn_window_event(
    event: On<WorldInspectorWindowEvent>,
    mut commands: Commands,
    query: Query<Entity, With<WorldInspectorWindow>>,
    asset_server: Res<AssetServer>,
) {
    if let WorldInspectorWindowEvent::SpawnWindow = *event {
        if query.iter().next().is_none() {
            let window = commands
                .spawn((
                    Name::new("World Inspector Window"),
                    Window {
                        title: "World Inspector".to_string(),
                        resolution: WindowResolution::new(DEFAULT_SIZE.x, DEFAULT_SIZE.y),
                        ..default()
                    },
                    WindowIcon {
                        handle: asset_server.load(MyTexture::WorldInspectorIcon),
                    },
                    WorldInspectorWindow,
                    PersistenceKey::<WorldInspectorWindowPersistenceProperty>::new(
                        "preferences/world_inspector_window.ron",
                    ),
                    PersistenceLoad::<WorldInspectorWindowPersistenceProperty>::default(),
                ))
                .id();
            commands.spawn((
                Name::new("World Inspector Window Camera"),
                Camera {
                    target: RenderTarget::Window(WindowRef::Entity(window)),
                    ..default()
                },
                Camera2d,
                WorldInspectorWindowCamera,
                EguiMultipassSchedule::new(WorldInspectorWindowEguiContextPass),
            ));
            debug!("World Inspector window spawned");
        } else {
            debug!("World Inspector window already exists, not spawning again");
        }
    }
}

fn handle_window_change(
    changed: Query<
        (
            Entity,
            &Window,
            Option<&PersistenceProperty<WorldInspectorWindowPersistenceProperty>>,
        ),
        Changed<Window>,
    >,
    mut commands: Commands,
) {
    for (entity, window, persistence) in changed.iter() {
        let new = WorldInspectorWindowPersistenceProperty::from(window).into_persistence_property();
        // Avoid change detection if nothing actually changed
        if let Some(old) = persistence
            && *old == new
        {
            continue;
        }

        commands.entity(entity).insert(new);
    }
}

fn handle_despawn_window_event(
    event: On<WorldInspectorWindowEvent>,
    mut commands: Commands,
    query: Query<Entity, With<WorldInspectorWindow>>,
) {
    if let WorldInspectorWindowEvent::DespawnWindow = *event {
        if let Some(entity) = query.iter().next() {
            commands.entity(entity).despawn();
            info!("World Inspector window despawned");
        }
    }
}

fn handle_toggle_window_event(
    event: On<WorldInspectorWindowEvent>,
    mut commands: Commands,
    query: Query<Entity, With<WorldInspectorWindow>>,
) {
    if let WorldInspectorWindowEvent::ToggleWindow = *event {
        if query.is_empty() {
            commands.trigger(WorldInspectorWindowEvent::SpawnWindow);
        } else {
            commands.trigger(WorldInspectorWindowEvent::DespawnWindow);
        }
    }
}

/// Copied from bevy-inspector-egui/src/quick.rs
fn check_plugins(app: &App, name: &str) {
    if !app.is_plugin_added::<MainSchedulePlugin>() {
        panic!(
            r#"`{name}` should be added after the default plugins:
        .add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin {{ .. }})
        .add_plugins({name}::default())
            "#,
        );
    }

    if !app.is_plugin_added::<EguiPlugin>() {
        panic!(
            r#"`{name}` needs to be added after `EguiPlugin`:
        .add_plugins(EguiPlugin {{ enable_multipass_for_primary_context: true }})
        .add_plugins({name}::default())
            "#,
        );
    }
}

fn handle_persistence_loaded(
    event: On<PersistenceLoaded<WorldInspectorWindowPersistenceProperty>>,
    mut windows: Query<&mut Window, With<WorldInspectorWindow>>,
    mut commands: Commands,
) {
    if let Ok(mut window) = windows.get_mut(event.entity) {
        info!(
            ?event,
            "Applying loaded persistence data to window"
        );
        window.position = event.property.position;
        window.resolution = event.property.resolution.clone();

        // Insert the property so it can be tracked for changes
        commands.entity(event.entity).insert(event.property.clone());
    }
}

fn ui(world: &mut World) {
    for mut egui_context in world
        .query_filtered::<&mut EguiContext, With<WorldInspectorWindowCamera>>()
        .iter_mut(world)
        .map(|ctx| ctx.clone())
        .collect_vec()
    {
        let ctx = egui_context.get_mut();
        // ctx.set_zoom_factor(2.0);
        // ctx.set_pixels_per_point(2.0);

        egui::CentralPanel::default().show(ctx, |ui| {
            // Optional: Add zoom controls at the top
            ui.horizontal(|ui| {
                zoom_menu_buttons(ui);
            });

            egui::ScrollArea::both().show(ui, |ui| {
                bevy_inspector::ui_for_world(world, ui);
                ui.allocate_space(ui.available_size());
            });
        });
    }
}
