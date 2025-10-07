use bevy::{
    app::{App, Plugin, Update},
    asset::{load_internal_asset, uuid_handle, Asset, Assets, Handle},
    diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin},
    ecs::system::{Res, ResMut},
    reflect::TypePath,
    render::{
        render_resource::{AsBindGroup, ShaderType},
        storage::ShaderStorageBuffer,
    },
    shader::{Shader, ShaderRef},
};
use bevy_ui_render::prelude::{UiMaterial, UiMaterialPlugin};

use super::FpsWindowConfig;

const FRAME_TIME_GRAPH_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("d85866b5-5f01-4c68-9a47-854d3141d689");

/// Plugin that sets up everything to render the frame time graph material.
pub struct FrameTimeGraphPlugin;

impl Plugin for FrameTimeGraphPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            FRAME_TIME_GRAPH_SHADER_HANDLE,
            "frame_time_graph.wgsl",
            Shader::from_wgsl
        );

        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }

        app.add_plugins(UiMaterialPlugin::<FrametimeGraphMaterial>::default())
            .add_systems(Update, update_frame_time_values);
    }
}

/// The config values sent to the frame time graph shader.
#[derive(Debug, Clone, Copy, ShaderType)]
pub struct FrameTimeGraphConfigUniform {
    /// minimum expected delta time
    dt_min: f32,
    /// maximum expected delta time
    dt_max: f32,
    dt_min_log2: f32,
    dt_max_log2: f32,
    /// controls whether or not the bars width are proportional to their delta time
    proportional_width: u32,
}

impl FrameTimeGraphConfigUniform {
    /// `proportional_width`: controls whether or not the bars width are proportional to their delta time
    pub fn new(target_fps: f32, min_fps: f32, proportional_width: bool) -> Self {
        // we want an upper limit that is above the target otherwise the bars will disappear
        let dt_min = 1.0 / (target_fps.max(1.0) * 1.2);
        let dt_max = 1.0 / min_fps.max(1.0);
        Self {
            dt_min,
            dt_max,
            dt_min_log2: dt_min.log2(),
            dt_max_log2: dt_max.log2(),
            proportional_width: u32::from(proportional_width),
        }
    }
}

/// The material used to render the frame time graph ui node.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct FrametimeGraphMaterial {
    /// The history of the previous frame times value.
    #[storage(0, read_only)]
    pub values: Handle<ShaderStorageBuffer>,
    /// The configuration values used by the shader to control how the graph is rendered.
    #[uniform(1)]
    pub config: FrameTimeGraphConfigUniform,
}

impl UiMaterial for FrametimeGraphMaterial {
    fn fragment_shader() -> ShaderRef {
        FRAME_TIME_GRAPH_SHADER_HANDLE.into()
    }
}

/// A system that updates the frame time values sent to the frame time graph.
fn update_frame_time_values(
    mut frame_time_graph_materials: ResMut<Assets<FrametimeGraphMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    diagnostics_store: Res<DiagnosticsStore>,
    config: Option<Res<FpsWindowConfig>>,
) {
    if config
        .as_ref()
        .is_some_and(|c| !c.frame_time_graph_config.enabled)
    {
        return;
    }

    let Some(frame_time) = diagnostics_store.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME) else {
        return;
    };

    let frame_times: Vec<f32> = frame_time.values().map(|x| *x as f32 / 1000.0).collect();

    for (_, material) in frame_time_graph_materials.iter_mut() {
        if let Some(buffer) = buffers.get_mut(&material.values) {
            buffer.set_data(frame_times.clone());
        }
    }
}

impl From<&FpsWindowConfig> for FrameTimeGraphConfigUniform {
    fn from(config: &FpsWindowConfig) -> Self {
        FrameTimeGraphConfigUniform::new(
            config.frame_time_graph_config.target_fps,
            config.frame_time_graph_config.min_fps,
            true,
        )
    }
}
