use crate::engine::assets::textures::MyTexture;
use crate::engine::window_persistence_plugin::PersistWindowProperties;
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

impl Plugin for MyWorldInspectorPlugin {
    fn build(&self, app: &mut App) {
        check_plugins(app, "WorldInspectorPlugin");
        app.add_plugins(DefaultInspectorConfigPlugin);
        app.add_systems(
            WorldInspectorWindowEguiContextPass,
            ui, // .run_if(|entities: Query<()>| entities.count() < 10_000),
        );
        app.add_observer(handle_spawn_window_event);
        app.add_observer(handle_despawn_window_event);
        app.add_observer(handle_toggle_window_event);
        app.add_observer(handle_camera_cleanup);
        app.add_systems(Startup, |mut commands: Commands| {
            // Open the window
            commands.trigger(WorldInspectorWindowEvent::SpawnWindow);
        });
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
                    PersistWindowProperties::new("preferences/world_inspector_window.ron"),
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

/// Remove cameras for windows which no longer exist
fn handle_camera_cleanup(
    removed_windows: On<Remove, WorldInspectorWindow>,
    mut commands: Commands,
    cameras: Query<(Entity, &Camera), With<WorldInspectorWindowCamera>>,
) {
    for (entity, camera) in cameras.iter() {
        if matches!(camera.target, RenderTarget::Window(WindowRef::Entity(e)) if e == removed_windows.entity)
        {
            commands.entity(entity).despawn();
            info!("World Inspector window camera despawned due to window being closed");
        }
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
        .add_plugins(EguiPlugin::default())
        .add_plugins({name}::default())
            "#,
        );
    }
}

fn ui(world: &mut World) {
    for mut egui_context in world
        .query_filtered::<&mut EguiContext, With<WorldInspectorWindowCamera>>()
        .iter_mut(world)
        .map(|ctx| ctx.clone())
        .collect_vec()
    {
        let _context_span = info_span!("egui_context_ui").entered();
        let ctx = egui_context.get_mut();
        // ctx.set_zoom_factor(2.0);
        // ctx.set_pixels_per_point(2.0);

        egui::CentralPanel::default().show(ctx, |ui| {
            // Optional: Add zoom controls at the top
            ui.horizontal(|ui| {
                zoom_menu_buttons(ui);
            });

            bevy_inspector::ui_for_world(world, ui);
        });
    }
}
