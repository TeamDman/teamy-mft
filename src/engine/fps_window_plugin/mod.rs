mod frame_time_graph;

use std::time::Duration;

use bevy::{
    camera::RenderTarget,
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    prelude::*,
    render::storage::ShaderStorageBuffer,
    text::{TextColor, TextFont, TextSpan},
    time::Time,
    ui::{
        widget::TextUiWriter, AlignItems, Display, FlexDirection, GlobalZIndex, JustifyContent, Node,
        PositionType, UiTargetCamera, Val,
    },
    window::{Window, WindowRef, WindowResolution},
};
use bevy_ui_render::prelude::MaterialNode;

use crate::engine::persistence_plugin::{
    Persistable, PersistenceKey, PersistenceLoad, PersistenceLoaded, PersistencePlugin,
    PersistenceProperty,
};

pub use frame_time_graph::{FrameTimeGraphConfigUniform, FrameTimeGraphPlugin, FrametimeGraphMaterial};

use self::frame_time_graph::FrameTimeGraphConfigUniform as GraphUniform;

/// [`GlobalZIndex`] used to render the fps overlay window contents.
pub const FPS_WINDOW_ZINDEX: i32 = i32::MAX - 16;

#[derive(Default)]
pub struct FpsWindowPlugin {
    pub config: FpsWindowConfig,
}

impl Plugin for FpsWindowPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }

        if !app.is_plugin_added::<FrameTimeGraphPlugin>() {
            app.add_plugins(FrameTimeGraphPlugin);
        }

        app.register_type::<FpsWindow>()
            .add_plugins(PersistencePlugin::<FpsWindowPersistenceProperty>::default())
            .insert_resource(self.config.clone())
            .add_systems(Startup, setup_window)
            .add_systems(
                Update,
                (
                    update_text,
                    customize_overlay.run_if(resource_changed::<FpsWindowConfig>),
                    update_visibility.run_if(resource_changed::<FpsWindowConfig>),
                    toggle_graph_display.run_if(resource_changed::<FpsWindowConfig>),
                    sync_graph_config.run_if(resource_changed::<FpsWindowConfig>),
                    handle_window_change,
                ),
            )
            .add_observer(handle_persistence_loaded);
    }
}

/// Configuration options for the FPS overlay window.
#[derive(Resource, Clone)]
pub struct FpsWindowConfig {
    /// Configuration of text in the overlay.
    pub text_config: TextFont,
    /// Color of text in the overlay.
    pub text_color: Color,
    /// Displays the FPS overlay window if true.
    pub enabled: bool,
    /// The period after which the FPS overlay re-renders.
    pub refresh_interval: Duration,
    /// Configuration of the frame time graph.
    pub frame_time_graph_config: FrameTimeGraphConfig,
    /// Title used when creating the dedicated window.
    pub window_title: String,
    /// Initial window resolution.
    pub window_resolution: WindowResolution,
}

impl Default for FpsWindowConfig {
    fn default() -> Self {
        Self {
            text_config: TextFont {
                font_size: 32.0,
                ..default()
            },
            text_color: Color::srgb(0.0, 1.0, 0.0),
            enabled: true,
            refresh_interval: Duration::from_millis(100),
            frame_time_graph_config: FrameTimeGraphConfig::default(),
            window_title: "FPS Overlay".to_owned(),
            window_resolution: WindowResolution::new(640, 360),
        }
    }
}

/// Configuration of the frame time graph.
#[derive(Clone, Copy)]
pub struct FrameTimeGraphConfig {
    /// Is the graph visible.
    pub enabled: bool,
    /// The minimum acceptable FPS.
    pub min_fps: f32,
    /// The target FPS.
    pub target_fps: f32,
}

impl FrameTimeGraphConfig {
    /// Constructs a default config for a given target fps.
    pub fn target_fps(target_fps: f32) -> Self {
        Self {
            enabled: true,
            min_fps: target_fps / 2.0,
            target_fps,
        }
    }
}

impl Default for FrameTimeGraphConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_fps: 30.0,
            target_fps: 144.0,
        }
    }
}

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct FpsWindow;

#[derive(Component)]
struct FpsText;

#[derive(Component)]
struct FrameTimeGraph;

#[derive(Component, Clone)]
struct FpsGraphMaterialHandle(pub Handle<FrametimeGraphMaterial>);

#[derive(Debug, Reflect, PartialEq, Clone)]
struct FpsWindowPersistenceProperty {
    position: WindowPosition,
    resolution: WindowResolution,
}

impl Persistable for FpsWindowPersistenceProperty {}

impl From<&Window> for FpsWindowPersistenceProperty {
    fn from(window: &Window) -> Self {
        Self {
            position: window.position,
            resolution: window.resolution.clone(),
        }
    }
}

fn setup_window(
    mut commands: Commands,
    existing: Query<Entity, With<FpsWindow>>,
    mut frame_time_graph_materials: ResMut<Assets<FrametimeGraphMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    config: Res<FpsWindowConfig>,
) {
    if !existing.is_empty() {
        return;
    }

    let window_entity = commands
        .spawn((
            Name::new("FPS Overlay Window"),
            Window {
                title: config.window_title.clone(),
                resolution: config.window_resolution.clone(),
                visible: config.enabled,
                resizable: true,
                decorations: true,
                ..default()
            },
            FpsWindow,
            PersistenceKey::<FpsWindowPersistenceProperty>::new("preferences/fps_window.ron"),
            PersistenceLoad::<FpsWindowPersistenceProperty>::default(),
        ))
        .id();

    let camera_entity = commands
        .spawn((
            Name::new("FPS Overlay Camera"),
            Camera2d,
            Camera {
                target: RenderTarget::Window(WindowRef::Entity(window_entity)),
                order: 100,
                ..default()
            },
        ))
        .id();

    let material_handle = frame_time_graph_materials.add(FrametimeGraphMaterial {
        values: buffers.add(ShaderStorageBuffer {
            data: Some(vec![0, 0, 0, 0]),
            ..Default::default()
        }),
        config: GraphUniform::from(config.as_ref()),
    });

    let root = commands
        .spawn((
            Name::new("FPS Overlay UI Root"),
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            UiTargetCamera(camera_entity),
            GlobalZIndex(FPS_WINDOW_ZINDEX),
        ))
        .id();

    commands.entity(root).with_children(|parent| {
        parent
            .spawn((
                Name::new("Frame Time Graph"),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    right: Val::Px(0.0),
                    bottom: Val::Px(0.0),
                    ..default()
                },
                MaterialNode::from(material_handle.clone()),
                GlobalZIndex(FPS_WINDOW_ZINDEX - 1),
                FrameTimeGraph,
                FpsGraphMaterialHandle(material_handle.clone()),
            ));

        parent
            .spawn((
                Name::new("FPS Text"),
                Text::new("FPS: "),
                config.text_config.clone(),
                TextColor(config.text_color),
                FpsText,
                GlobalZIndex(FPS_WINDOW_ZINDEX + 1),
            ))
            .with_child((TextSpan::from("--"), config.text_config.clone()));
    });
}

fn update_text(
    diagnostic: Res<DiagnosticsStore>,
    query: Query<Entity, With<FpsText>>,
    mut writer: TextUiWriter,
    time: Res<Time>,
    config: Res<FpsWindowConfig>,
    mut time_since_rerender: Local<Duration>,
) {
    *time_since_rerender += time.delta();
    if *time_since_rerender < config.refresh_interval {
        return;
    }
    *time_since_rerender = Duration::ZERO;

    let Some(fps) = diagnostic
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|fps| fps.smoothed())
    else {
        return;
    };

    let value = format!("{fps:>6.2}");
    for entity in &query {
        *writer.text(entity, 1) = value.clone();
    }
}

fn customize_overlay(
    config: Res<FpsWindowConfig>,
    mut writer: TextUiWriter,
    query: Query<Entity, With<FpsText>>,
) {
    for entity in &query {
        writer.for_each_font(entity, |mut font| {
            *font = config.text_config.clone();
        });
        writer.for_each_color(entity, |mut color| color.0 = config.text_color);
    }
}

fn update_visibility(
    config: Res<FpsWindowConfig>,
    mut windows: Query<&mut Window, With<FpsWindow>>,
) {
    for mut window in &mut windows {
        window.visible = config.enabled;
    }
}

fn toggle_graph_display(
    config: Res<FpsWindowConfig>,
    mut graphs: Query<&mut Node, With<FrameTimeGraph>>,
) {
    for mut node in &mut graphs {
        node.display = if config.frame_time_graph_config.enabled {
            Display::DEFAULT
        } else {
            Display::None
        };
    }
}

fn sync_graph_config(
    config: Res<FpsWindowConfig>,
    mut materials: ResMut<Assets<FrametimeGraphMaterial>>,
    graphs: Query<&FpsGraphMaterialHandle>,
) {
    for handle in &graphs {
        if let Some(material) = materials.get_mut(&handle.0) {
            material.config = GraphUniform::from(config.as_ref());
        }
    }
}

fn handle_window_change(
    changed: Query<
        (
            Entity,
            &Window,
            Option<&PersistenceProperty<FpsWindowPersistenceProperty>>,
        ),
        (Changed<Window>, With<FpsWindow>),
    >,
    mut commands: Commands,
) {
    for (entity, window, persistence) in &changed {
        let new = FpsWindowPersistenceProperty::from(window).into_persistence_property();
        if let Some(old) = persistence && *old == new {
            continue;
        }

        commands.entity(entity).insert(new);
    }
}

fn handle_persistence_loaded(
    event: On<PersistenceLoaded<FpsWindowPersistenceProperty>>,
    mut windows: Query<&mut Window, With<FpsWindow>>,
    mut commands: Commands,
) {
    if let Ok(mut window) = windows.get_mut(event.entity) {
        info!(?event, "Applying loaded persistence data to FPS window");
        window.position = event.property.position;
        window.resolution = event.property.resolution.clone();

        commands.entity(event.entity).insert(event.property.clone());
    }
}
