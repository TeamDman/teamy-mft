use crate::DEFAULT_EXTRA_FILTERS;
use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::engine::camera_controller_plugin::CameraController;
use crate::engine::camera_controller_plugin::CameraControllerPlugin;
use crate::engine::camera_controller_plugin::CameraFocusController;
use crate::engine::camera_controller_plugin::FocusTarget;
use crate::engine::shimmer_material_plugin::ShimmerMaterial;
use crate::engine::shimmer_material_plugin::ShimmerMaterialPlugin;
use crate::engine::shimmer_material_plugin::shimmer_material;
use arbitrary::Arbitrary;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::log::LogPlugin;
use bevy::pbr::MeshMaterial3d;
use bevy::picking::prelude::MeshPickingPlugin;
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy::window::ExitCondition;
use bevy::window::WindowPlugin;
use bevy::window::WindowResolution;
use bevy_mesh_outline::MeshOutlinePlugin;
use bevy_mesh_outline::OutlineCamera;
use clap::Args;
use tracing::debug;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct OrbitDemoArgs {}

impl OrbitDemoArgs {
    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        debug!("Building orbit demo");
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Orbit Demo".into(),
                        resolution: WindowResolution::new(1280, 720),
                        ..default()
                    }),
                    exit_condition: ExitCondition::OnAllClosed,
                    ..default()
                })
                .set(LogPlugin {
                    level: global_args.log_level().into(),
                    filter: DEFAULT_EXTRA_FILTERS.to_string(),
                    ..default()
                }),
        );
        app.add_plugins(MeshPickingPlugin);
        app.add_plugins(MeshOutlinePlugin);
        app.add_plugins(ShimmerMaterialPlugin);
        app.add_plugins(CameraControllerPlugin);

        app.insert_resource(AmbientLight {
            color: Color::srgb(0.4, 0.4, 0.45),
            brightness: 150.0,
            ..default()
        });

        app.add_systems(Startup, (spawn_orbit_demo_camera, spawn_orbit_demo_scene));

        debug!("Orbit demo ready, running app");
        app.run();
        Ok(())
    }
}

impl ToArgs for OrbitDemoArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        vec![]
    }
}

fn spawn_orbit_demo_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("Orbit Demo Camera"),
        Camera::default(),
        Camera3d::default(),
        CameraController::default(),
        CameraFocusController::default(),
        OutlineCamera,
        DepthPrepass,
        Msaa::Off,
        Transform::from_xyz(-8.0, 5.0, 11.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_orbit_demo_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shimmer_materials: ResMut<Assets<ShimmerMaterial>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Orbit Demo Directional Light"),
        DirectionalLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(6.0, 12.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Name::new("Orbit Demo Ground"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
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
        &mut shimmer_materials,
        Mesh::from(Cuboid::new(1.8, 1.2, 1.0)),
        Color::srgb(0.95, 0.35, 0.45),
        Transform::from_xyz(-7.5, 0.6, -1.5),
        "Shimmer Cube",
    );

    spawn_focus_target(
        &mut commands,
        &mut meshes,
        &mut shimmer_materials,
        Mesh::from(Sphere::new(0.9)),
        Color::srgb(0.2, 0.75, 0.9),
        Transform::from_xyz(-5.0, 1.1, 0.0),
        "Shimmer Sphere",
    );

    spawn_focus_target(
        &mut commands,
        &mut meshes,
        &mut shimmer_materials,
        Mesh::from(Capsule3d::new(0.35, 0.75)),
        Color::srgb(0.9, 0.8, 0.2),
        Transform::from_xyz(-2.25, 0.9, 1.75).with_rotation(Quat::from_rotation_z(0.4)),
        "Shimmer Capsule",
    );
}

fn spawn_focus_target(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    shimmer_materials: &mut Assets<ShimmerMaterial>,
    mesh: Mesh,
    color: Color,
    transform: Transform,
    name: &'static str,
) {
    commands.spawn((
        Name::new(name),
        FocusTarget,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(shimmer_materials.add(shimmer_material(color))),
        transform,
    ));
}
