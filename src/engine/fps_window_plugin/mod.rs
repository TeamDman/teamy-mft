mod frame_time_graph;

use self::frame_time_graph::FrameTimeGraphConfigUniform as GraphUniform;
use crate::engine::persistence_plugin::Persistable;
use crate::engine::persistence_plugin::PersistenceKey;
use crate::engine::persistence_plugin::PersistenceLoad;
use crate::engine::persistence_plugin::PersistenceLoaded;
use crate::engine::persistence_plugin::PersistencePlugin;
use crate::engine::persistence_plugin::PersistenceProperty;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::diagnostic::DiagnosticsStore;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::render::render_resource::TextureDimension;
use bevy::render::render_resource::TextureFormat;
use bevy::render::storage::ShaderStorageBuffer;
use bevy::text::TextColor;
use bevy::text::TextFont;
use bevy::text::TextSpan;
use bevy::time::Time;
use bevy::ui::AlignItems;
use bevy::ui::Display;
use bevy::ui::FlexDirection;
use bevy::ui::GlobalZIndex;
use bevy::ui::JustifyContent;
use bevy::ui::Node;
use bevy::ui::PositionType;
use bevy::ui::UiTargetCamera;
use bevy::ui::Val;
use bevy::ui::widget::TextUiWriter;
use bevy::window::Window;
use bevy::window::WindowIcon;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;
use bevy_ui_render::prelude::MaterialNode;
pub use frame_time_graph::FrameTimeGraphConfigUniform;
pub use frame_time_graph::FrameTimeGraphPlugin;
pub use frame_time_graph::FrametimeGraphMaterial;
use std::time::Duration;

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
                    update_window_icon
                        .run_if(|config: Res<FpsWindowConfig>| config.icon_config.enabled),
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
    /// Configuration for the generated window icon.
    pub icon_config: FpsWindowIconConfig,
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
            icon_config: FpsWindowIconConfig::default(),
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

#[derive(Clone)]
pub struct FpsWindowIconConfig {
    pub enabled: bool,
    pub bar_count: usize,
    pub time_window: Duration,
    pub refresh_interval: Duration,
}

impl Default for FpsWindowIconConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bar_count: 16,
            time_window: Duration::from_secs(2),
            refresh_interval: Duration::from_millis(250),
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

#[derive(Component, Clone)]
struct FpsIconImageHandle(pub Handle<Image>);

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
    mut images: ResMut<Assets<Image>>,
    config: Res<FpsWindowConfig>,
) {
    if !existing.is_empty() {
        return;
    }

    let icon_dimensions = icon_dimensions(&config.icon_config);
    let initial_icon_data = vec![0u8; (icon_dimensions.0 * icon_dimensions.1 * 4) as usize];
    let icon_image = create_icon_image(icon_dimensions.0, icon_dimensions.1, &initial_icon_data);
    let icon_handle = images.add(icon_image);

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
            WindowIcon {
                handle: icon_handle.clone(),
            },
            FpsIconImageHandle(icon_handle.clone()),
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
        parent.spawn((
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

const ICON_HEIGHT: u32 = 32;

fn icon_dimensions(config: &FpsWindowIconConfig) -> (u32, u32) {
    let width = config.bar_count.max(1) as u32;
    (width, ICON_HEIGHT)
}

fn create_icon_image(width: u32, height: u32, data: &[u8]) -> Image {
    Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
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
        if let Some(old) = persistence
            && *old == new
        {
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

fn update_window_icon(
    diagnostics: Res<DiagnosticsStore>,
    config: Res<FpsWindowConfig>,
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    query: Query<&FpsIconImageHandle, With<FpsWindow>>,
    mut time_since_update: Local<Duration>,
) {
    *time_since_update += time.delta();
    if *time_since_update < config.icon_config.refresh_interval {
        return;
    }
    *time_since_update = Duration::ZERO;

    let Some(icon_handle) = query.iter().next() else {
        return;
    };

    let Some(image) = images.get_mut(&icon_handle.0) else {
        return;
    };

    let Some(frame_time) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME) else {
        return;
    };

    let values: Vec<f32> = frame_time.values().map(|ms| *ms as f32).collect();
    if values.is_empty() {
        return;
    }

    let time_window_ms = config.icon_config.time_window.as_secs_f32() * 1000.0;
    if time_window_ms <= f32::EPSILON {
        return;
    }

    let (width, height) = icon_dimensions(&config.icon_config);

    let bar_count = config.icon_config.bar_count.max(1);
    let bucket_duration = time_window_ms / bar_count as f32;
    if bucket_duration <= f32::EPSILON {
        return;
    }

    let mut buckets: Vec<Vec<f32>> = vec![Vec::new(); bar_count];
    let mut elapsed = 0.0;
    for &dt_ms in values.iter().rev() {
        elapsed += dt_ms;
        if elapsed > time_window_ms {
            break;
        }
        let index = ((elapsed / bucket_duration).floor() as usize).min(bar_count - 1);
        buckets[index].push(dt_ms);
    }

    let mut bar_heights = Vec::with_capacity(bar_count);
    let mut bar_colors = Vec::with_capacity(bar_count);
    let mut last_dt = values.last().copied().unwrap_or(16.0);
    let min_fps = config.frame_time_graph_config.min_fps.max(1.0);
    let target_fps = config
        .frame_time_graph_config
        .target_fps
        .max(min_fps + f32::EPSILON);

    for bucket in buckets {
        let dt = if bucket.is_empty() {
            last_dt
        } else {
            let sum: f32 = bucket.iter().sum();
            let avg = sum / bucket.len() as f32;
            last_dt = avg;
            avg
        };

        let fps = if dt > f32::EPSILON {
            1000.0 / dt
        } else {
            target_fps
        };
        let height_ratio = (fps / target_fps).clamp(0.0, 1.0);
        bar_heights.push(height_ratio);

        let color_ratio = ((fps - min_fps) / (target_fps - min_fps)).clamp(0.0, 1.0);
        let color = [
            ((1.0 - color_ratio) * 255.0) as u8,
            (color_ratio * 255.0) as u8,
            0,
            255,
        ];
        bar_colors.push(color);
    }

    let mut data = vec![0u8; (width * height * 4) as usize];
    for (x, (&ratio, color)) in bar_heights.iter().zip(bar_colors.iter()).enumerate() {
        let bar_height = (ratio * height as f32).round() as u32;
        for y in 0..height {
            let offset = ((y * width + x as u32) * 4) as usize;
            if height - y <= bar_height {
                data[offset..offset + 4].copy_from_slice(color);
            } else {
                data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }

    *image = create_icon_image(width, height, &data);
}
