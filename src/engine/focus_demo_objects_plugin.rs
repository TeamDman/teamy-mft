use crate::engine::camera_controller_plugin::FocusTarget;
use crate::engine::camera_controller_plugin::clear_hover_on_exit;
use crate::engine::camera_controller_plugin::store_hover_on_enter;
use bevy::math::Vec4;
use bevy::pbr::ExtendedMaterial;
use bevy::pbr::MaterialExtension;
use bevy::pbr::MaterialPlugin;
use bevy::pbr::MeshMaterial3d;
use bevy::pbr::OpaqueRendererMethod;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub const SHADER_ASSET_PATH: &str = "shaders/rainbow_outline.wgsl";

/// Custom StandardMaterial + shader extension used to draw the rainbow glow.
pub type GlowMaterial = ExtendedMaterial<StandardMaterial, RainbowOutlineExtension>;

pub struct FocusDemoObjectsPlugin;

impl Plugin for FocusDemoObjectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<GlowMaterial>::default());
        app.add_systems(Startup, spawn_focus_demo_content);
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct RainbowOutlineExtension {
    #[uniform(100)]
    glow_control: Vec4,
}

impl Default for RainbowOutlineExtension {
    fn default() -> Self {
        Self {
            glow_control: Vec4::new(0.0, 0.0, 2.5, 0.35),
        }
    }
}

impl RainbowOutlineExtension {
    pub fn set_glow_strength(&mut self, strength: f32) {
        self.glow_control.x = strength;
    }

    pub fn set_phase(&mut self, phase: f32) {
        self.glow_control.y = phase;
    }

    pub fn set_outline_width(&mut self, width: f32) {
        self.glow_control.w = width;
    }
}

impl MaterialExtension for RainbowOutlineExtension {
    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }
}

fn spawn_focus_demo_content(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut glow_materials: ResMut<Assets<GlowMaterial>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Focus Demo Directional Light"),
        DirectionalLight::default(),
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Name::new("Focus Demo Ground"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(standard_materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.05, 0.08),
            perceptual_roughness: 0.95,
            cull_mode: None,
            ..default()
        })),
    ));

    spawn_focus_target(
        &mut commands,
        &mut meshes,
        &mut glow_materials,
        Mesh::from(Cuboid::new(1.8, 1.2, 1.0)),
        Color::srgb(0.95, 0.35, 0.45),
        Transform::from_xyz(-7.5, 0.6, -1.5),
        "Glow Cube",
    );

    spawn_focus_target(
        &mut commands,
        &mut meshes,
        &mut glow_materials,
        Mesh::from(Sphere::new(0.9)),
        Color::srgb(0.2, 0.75, 0.9),
        Transform::from_xyz(-5.0, 1.1, 0.0),
        "Glow Sphere",
    );

    spawn_focus_target(
        &mut commands,
        &mut meshes,
        &mut glow_materials,
        Mesh::from(Capsule3d::new(0.35, 0.75)),
        Color::srgb(0.9, 0.8, 0.2),
        Transform::from_xyz(-2.25, 0.9, 1.75).with_rotation(Quat::from_rotation_z(0.4)),
        "Glow Capsule",
    );
}

fn spawn_focus_target(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    glow_materials: &mut Assets<GlowMaterial>,
    mesh: Mesh,
    color: Color,
    transform: Transform,
    name: &'static str,
) {
    commands
        .spawn((
            Name::new(name),
            FocusTarget,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(glow_materials.add(glow_material(color))),
            transform,
        ))
        .observe(store_hover_on_enter)
        .observe(clear_hover_on_exit);
}

fn glow_material(color: Color) -> GlowMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: color.into(),
            perceptual_roughness: 0.5,
            opaque_render_method: OpaqueRendererMethod::Auto,
            ..default()
        },
        extension: RainbowOutlineExtension::default(),
    }
}
