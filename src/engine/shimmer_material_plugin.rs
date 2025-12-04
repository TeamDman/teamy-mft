use bevy::math::Vec4;
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::MaterialExtension;
use bevy::pbr::MaterialPlugin;
use bevy::pbr::OpaqueRendererMethod;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub const SHIMMER_SHADER_ASSET_PATH: &str = "shaders/rainbow_outline.wgsl";

/// Custom StandardMaterial + shader extension used to draw the shimmer highlight.
pub type ShimmerMaterial = ExtendedMaterial<StandardMaterial, ShimmerMaterialExtension>;

pub struct ShimmerMaterialPlugin;

impl Plugin for ShimmerMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ShimmerMaterial>::default());
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ShimmerMaterialExtension {
    #[uniform(100)]
    glow_control: Vec4,
}

impl Default for ShimmerMaterialExtension {
    fn default() -> Self {
        Self {
            glow_control: Vec4::new(0.0, 0.0, 2.5, 0.35),
        }
    }
}

impl ShimmerMaterialExtension {
    pub fn set_shimmer_strength(&mut self, strength: f32) {
        self.glow_control.x = strength;
    }

    pub fn set_phase(&mut self, phase: f32) {
        self.glow_control.y = phase;
    }

    pub fn set_band_spacing(&mut self, spacing: f32) {
        self.glow_control.z = spacing;
    }

    pub fn set_outline_width(&mut self, width: f32) {
        self.glow_control.w = width;
    }
}

impl MaterialExtension for ShimmerMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        SHIMMER_SHADER_ASSET_PATH.into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        SHIMMER_SHADER_ASSET_PATH.into()
    }
}

pub fn shimmer_material(color: Color) -> ShimmerMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: color.into(),
            perceptual_roughness: 0.5,
            opaque_render_method: OpaqueRendererMethod::Auto,
            ..default()
        },
        extension: ShimmerMaterialExtension::default(),
    }
}
