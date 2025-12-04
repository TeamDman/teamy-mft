use crate::engine::camera_wasd_controller_plugin::CameraController;
use crate::engine::camera_wasd_controller_plugin::CameraControllerSyncExt;
use crate::engine::shimmer_material_plugin::ShimmerMaterial;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::mouse::MouseScrollUnit;
use bevy::input::mouse::MouseWheel;
use bevy::math::Vec3Swizzles;
use bevy::pbr::MeshMaterial3d;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy_mesh_outline::MeshOutline;
use std::f32::consts::FRAC_PI_2;

/// Handles orbit-style camera focus and highlighting.
pub struct CameraOrbitControllerPlugin;

impl Plugin for CameraOrbitControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                focus_on_hovered_entity,
                apply_scroll_zoom,
                update_focus_camera,
                drive_shimmer_materials,
                drive_hover_outlines,
            ),
        );
    }
}

/// Marker component applied to meshes that can be focused.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct FocusTarget;

/// Camera focus controller component that stores orbit configuration/state.
#[derive(Component)]
pub struct CameraFocusController {
    focus: Option<Entity>,
    yaw: f32,
    pitch: f32,
    distance: f32,
    min_distance: f32,
    max_distance: f32,
    orbit_sensitivity: Vec2,
    zoom_sensitivity: f32,
    pitch_limit: f32,
}

impl Default for CameraFocusController {
    fn default() -> Self {
        Self {
            focus: None,
            yaw: 0.0,
            pitch: -0.2,
            distance: 10.0,
            min_distance: 1.0,
            max_distance: 30.0,
            orbit_sensitivity: Vec2::new(0.01, 0.008),
            zoom_sensitivity: 1.0,
            pitch_limit: FRAC_PI_2 - 0.05,
        }
    }
}

impl CameraFocusController {
    pub fn focus(&self) -> Option<Entity> {
        self.focus
    }

    pub fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    pub fn focus_on(
        &mut self,
        target: Entity,
        camera_transform: &Transform,
        target_position: Vec3,
    ) {
        self.focus = Some(target);
        let offset = camera_transform.translation - target_position;
        let horizontal = offset.xz().length().max(f32::EPSILON);
        self.distance = offset.length().clamp(self.min_distance, self.max_distance);
        self.yaw = offset.x.atan2(offset.z);
        self.pitch = offset
            .y
            .atan2(horizontal)
            .clamp(-self.pitch_limit, self.pitch_limit);
    }

    pub fn release_focus(&mut self, transform: &Transform) {
        self.sync_from_transform(transform);
        self.focus = None;
    }

    fn offset_vector(&self) -> Vec3 {
        let horizontal = self.distance * self.pitch.cos();
        let x = horizontal * self.yaw.sin();
        let z = horizontal * self.yaw.cos();
        let y = self.distance * self.pitch.sin();
        Vec3::new(x, y, z)
    }

    fn sync_from_transform(&mut self, transform: &Transform) {
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        self.pitch = pitch.clamp(-self.pitch_limit, self.pitch_limit);
    }
}

fn focus_on_hovered_entity(
    keys: Res<ButtonInput<KeyCode>>,
    hover_map: Option<Res<HoverMap>>,
    mut camera_query: Query<(
        &mut CameraFocusController,
        &Transform,
        Option<&mut CameraController>,
    )>,
    targets: Query<&GlobalTransform, With<FocusTarget>>,
) {
    if !keys.just_pressed(KeyCode::KeyF) {
        return;
    }

    let Ok((mut rig, camera_transform, camera_controller)) = camera_query.single_mut() else {
        return;
    };

    let hovered_target = hover_map
        .as_ref()
        .and_then(|map| hovered_focus_target(map, &targets));

    match (rig.focus(), hovered_target) {
        (Some(current), Some(new_target)) if current != new_target => {
            if let Ok(target_transform) = targets.get(new_target) {
                rig.focus_on(
                    new_target,
                    camera_transform,
                    target_transform.translation(),
                );
            }
        }
        (Some(_), _) => {
            if let Some(mut controller) = camera_controller {
                controller.sync_from_transform(camera_transform);
            }
            rig.release_focus(camera_transform);
        }
        (None, Some(target_entity)) => {
            if let Ok(target_transform) = targets.get(target_entity) {
                rig.focus_on(
                    target_entity,
                    camera_transform,
                    target_transform.translation(),
                );
            }
        }
        (None, None) => {}
    }
}

fn apply_scroll_zoom(
    mut camera_query: Query<&mut CameraFocusController>,
    mut mouse_wheel_reader: MessageReader<MouseWheel>,
) {
    let Ok(mut rig) = camera_query.single_mut() else {
        return;
    };

    if rig.focus.is_none() {
        return;
    }

    let mut scroll_delta = 0.0;
    for wheel in mouse_wheel_reader.read() {
        let unit = match wheel.unit {
            MouseScrollUnit::Line => 1.0,
            MouseScrollUnit::Pixel => 0.05,
        };
        scroll_delta += wheel.y * unit;
    }

    if scroll_delta.abs() > f32::EPSILON {
        rig.distance = (rig.distance - scroll_delta * rig.zoom_sensitivity)
            .clamp(rig.min_distance, rig.max_distance);
    }
}

fn update_focus_camera(
    mut query: Query<(
        &mut Transform,
        &mut CameraFocusController,
        Option<&mut CameraController>,
    )>,
    targets: Query<&GlobalTransform, With<FocusTarget>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let Ok((mut transform, mut rig, camera_controller)) = query.single_mut() else {
        return;
    };

    if keys.just_pressed(KeyCode::Escape) && rig.focus.is_some() {
        if let Some(mut controller) = camera_controller {
            controller.sync_from_transform(&transform);
        }
        rig.release_focus(&transform);
        return;
    }

    let Some(target_entity) = rig.focus else {
        return;
    };

    let Ok(target_transform) = targets.get(target_entity) else {
        if let Some(mut controller) = camera_controller {
            controller.sync_from_transform(&transform);
        }
        rig.release_focus(&transform);
        return;
    };

    if mouse_buttons.pressed(MouseButton::Right) {
        let delta = mouse_motion.delta;
        rig.yaw -= delta.x * rig.orbit_sensitivity.x;
        rig.pitch = (rig.pitch + delta.y * rig.orbit_sensitivity.y)
            .clamp(-rig.pitch_limit, rig.pitch_limit);
    }

    let target_position = target_transform.translation();
    transform.translation = target_position + rig.offset_vector();
    transform.look_at(target_position, Vec3::Y);
}

fn drive_shimmer_materials(
    time: Res<Time>,
    camera: Query<&CameraFocusController>,
    mut materials: ResMut<Assets<ShimmerMaterial>>,
    targets: Query<(Entity, &MeshMaterial3d<ShimmerMaterial>), With<FocusTarget>>,
) {
    let Ok(rig) = camera.single() else {
        return;
    };

    let phase = time.elapsed_secs();
    for (entity, material_handle) in &targets {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            material.extension.set_phase(phase);
            let shimmer_strength = if Some(entity) == rig.focus { 1.0 } else { 0.0 };
            material.extension.set_shimmer_strength(shimmer_strength);
            material.extension.set_outline_width(0.45);
        }
    }
}

fn drive_hover_outlines(
    mut commands: Commands,
    hover_map: Option<Res<HoverMap>>,
    targets: Query<&GlobalTransform, With<FocusTarget>>,
    mut outlined: Local<Option<Entity>>,
) {
    let hovered = hover_map
        .as_ref()
        .and_then(|map| hovered_focus_target(map, &targets));

    if hovered == *outlined {
        return;
    }

    if let Some(previous) = outlined.take() {
        if targets.get(previous).is_ok() {
            commands.entity(previous).remove::<MeshOutline>();
        }
    }

    if let Some(entity) = hovered {
        if targets.get(entity).is_ok() {
            commands.entity(entity).insert(default_hover_outline());
            *outlined = Some(entity);
        }
    }
}

fn default_hover_outline() -> MeshOutline {
    MeshOutline::new(8.0)
        .with_color(Color::srgb(0.9, 0.95, 1.0))
        .with_intensity(1.2)
        .with_priority(0.5)
}

fn hovered_focus_target(
    hover_map: &HoverMap,
    targets: &Query<&GlobalTransform, With<FocusTarget>>,
) -> Option<Entity> {
    let pointer_hits = hover_map.get(&PointerId::Mouse)?;
    pointer_hits
        .keys()
        .find(|entity| targets.get(**entity).is_ok())
        .copied()
}
