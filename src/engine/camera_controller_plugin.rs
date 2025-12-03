//! A freecam-style camera controller plugin with optional focus targeting.
//! To use in your own application:
//! - Copy the code for the [`CameraControllerPlugin`] and add the plugin to your App.
//! - Attach the [`CameraController`] component (and optionally [`CameraFocusController`]) to an
//!   entity with a [`Camera3d`].
//!
//! Unlike other examples, which demonstrate an application, this demonstrates a plugin library.

use crate::engine::focus_demo_objects_plugin::GlowMaterial;
use bevy::camera::RenderTarget;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::mouse::MouseScrollUnit;
use bevy::input::mouse::MouseWheel;
use bevy::math::Vec3Swizzles;
use bevy::pbr::MeshMaterial3d;
use bevy::picking::prelude::*;
use bevy::prelude::*;
use bevy::window::CursorGrabMode;
use bevy::window::CursorOptions;
use bevy::window::WindowRef;
use std::f32::consts::*;
use std::fmt;

/// A freecam-style camera controller plugin that also supports focusing hovered entities.
pub struct CameraControllerPlugin;

impl Plugin for CameraControllerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HoveredEntity>()
            .add_systems(Update, run_camera_controller)
            .add_systems(
                Update,
                (
                    focus_on_hovered_entity,
                    apply_scroll_zoom,
                    update_focus_camera,
                    drive_outline_materials,
                ),
            );
    }
}

/// Tracks the currently hovered entity reported by picking events.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct HoveredEntity(pub Option<Entity>);

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
            pitch_limit: std::f32::consts::FRAC_PI_2 - 0.05,
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

/// Based on Valorant's default sensitivity, not entirely sure why it is exactly 1.0 / 180.0,
/// but I'm guessing it is a misunderstanding between degrees/radians and then sticking with
/// it because it felt nice.
pub const RADIANS_PER_DOT: f32 = 1.0 / 180.0;

/// Camera controller [`Component`].
#[derive(Component)]
pub struct CameraController {
    /// Enables this [`CameraController`] when `true`.
    pub enabled: bool,
    /// Indicates if this controller has been initialized by the [`CameraControllerPlugin`].
    pub initialized: bool,
    /// Multiplier for pitch and yaw rotation speed.
    pub sensitivity: f32,
    /// [`KeyCode`] for forward translation.
    pub key_forward: KeyCode,
    /// [`KeyCode`] for backward translation.
    pub key_back: KeyCode,
    /// [`KeyCode`] for left translation.
    pub key_left: KeyCode,
    /// [`KeyCode`] for right translation.
    pub key_right: KeyCode,
    /// [`KeyCode`] for up translation.
    pub key_up: KeyCode,
    /// [`KeyCode`] for down translation.
    pub key_down: KeyCode,
    /// [`KeyCode`] to use [`run_speed`](CameraController::run_speed) instead of
    /// [`walk_speed`](CameraController::walk_speed) for translation.
    pub key_run: KeyCode,
    /// [`MouseButton`] for grabbing the mouse focus.
    pub mouse_key_cursor_grab: MouseButton,
    /// [`KeyCode`] for grabbing the keyboard focus.
    pub keyboard_key_toggle_cursor_grab: KeyCode,
    /// Multiplier for unmodified translation speed.
    pub walk_speed: f32,
    /// Multiplier for running translation speed.
    pub run_speed: f32,
    /// Multiplier for how the mouse scroll wheel modifies [`walk_speed`](CameraController::walk_speed)
    /// and [`run_speed`](CameraController::run_speed).
    pub scroll_factor: f32,
    /// Friction factor used to exponentially decay [`velocity`](CameraController::velocity) over time.
    pub friction: f32,
    /// This [`CameraController`]'s pitch rotation.
    pub pitch: f32,
    /// This [`CameraController`]'s yaw rotation.
    pub yaw: f32,
    /// This [`CameraController`]'s translation velocity.
    pub velocity: Vec3,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            enabled: true,
            initialized: false,
            sensitivity: 1.0,
            key_forward: KeyCode::KeyW,
            key_back: KeyCode::KeyS,
            key_left: KeyCode::KeyA,
            key_right: KeyCode::KeyD,
            key_up: KeyCode::KeyE,
            key_down: KeyCode::KeyQ,
            key_run: KeyCode::ShiftLeft,
            mouse_key_cursor_grab: MouseButton::Right,
            keyboard_key_toggle_cursor_grab: KeyCode::KeyM,
            walk_speed: 5.0,
            run_speed: 15.0,
            scroll_factor: 0.1,
            friction: 0.5,
            pitch: 0.0,
            yaw: 0.0,
            velocity: Vec3::ZERO,
        }
    }
}

impl fmt::Display for CameraController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "\nFreecam Controls:\n    Mouse\t- Move camera orientation\n    Scroll\t- Adjust movement speed\n    {:?}\t- Hold to grab cursor\n    {:?}\t- Toggle cursor grab\n    {:?} & {:?}\t- Fly forward & backwards\n    {:?} & {:?}\t- Fly sideways left & right\n    {:?} & {:?}\t- Fly up & down\n    {:?}\t- Fly faster while held",
            self.mouse_key_cursor_grab,
            self.keyboard_key_toggle_cursor_grab,
            self.key_forward,
            self.key_back,
            self.key_left,
            self.key_right,
            self.key_up,
            self.key_down,
            self.key_run,
        )
    }
}

fn run_camera_controller(
    time: Res<Time<Real>>,
    windows: Query<(Entity, &Window)>,
    mut cursor_options_query: Query<&mut CursorOptions>,
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    accumulated_mouse_scroll: Res<AccumulatedMouseScroll>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    key_input: Res<ButtonInput<KeyCode>>,
    mut toggle_cursor_grab: Local<bool>,
    mut mouse_cursor_grab: Local<bool>,
    mut query: Query<(
        Entity,
        &mut Transform,
        &mut CameraController,
        &Camera,
        Option<&CameraFocusController>,
    )>,
) {
    let dt = time.delta_secs();

    let Ok((entity, mut transform, mut controller, camera, focus)) = query.single_mut() else {
        return;
    };

    if !controller.initialized {
        let (yaw, pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
        controller.yaw = yaw;
        controller.pitch = pitch;
        controller.initialized = true;
        info!(entity = ?entity, "{}", *controller);
    }
    if !controller.enabled {
        return;
    }

    let target_window_entity = match &camera.target {
        RenderTarget::Window(window_ref) => match window_ref {
            WindowRef::Entity(window_entity) => Some(*window_entity),
            WindowRef::Primary => windows.iter().next().map(|(entity, _)| entity),
        },
        _ => None,
    };

    let mut window_focused = true;
    if let Some(window_entity) = target_window_entity {
        if let Some((_, window)) = windows
            .iter()
            .find(|(entity_id, _)| *entity_id == window_entity)
        {
            window_focused = window.focused;
        } else {
            debug!(controller = ?entity, window = ?window_entity, "Camera target window not found in query");
        }
    }

    let mut cursor_grab_change = false;

    if key_input.just_pressed(controller.keyboard_key_toggle_cursor_grab) {
        *toggle_cursor_grab = !*toggle_cursor_grab;
        cursor_grab_change = true;
        debug!(controller = ?entity, toggle = *toggle_cursor_grab, "Toggled cursor grab via keyboard");
    }

    let pressed_now = mouse_button_input.pressed(controller.mouse_key_cursor_grab);
    if pressed_now != *mouse_cursor_grab {
        *mouse_cursor_grab = pressed_now;
        cursor_grab_change = true;
        debug!(
            controller = ?entity,
            button = ?controller.mouse_key_cursor_grab,
            pressed = pressed_now,
            "Mouse grab button state changed"
        );
    }

    if !window_focused {
        if *toggle_cursor_grab || *mouse_cursor_grab {
            debug!(controller = ?entity, "Target window lost focus; resetting grab state");
        }
        if *toggle_cursor_grab {
            *toggle_cursor_grab = false;
            cursor_grab_change = true;
        }
        if *mouse_cursor_grab {
            *mouse_cursor_grab = false;
            cursor_grab_change = true;
        }
    }

    let cursor_grab = *toggle_cursor_grab || *mouse_cursor_grab;

    if cursor_grab_change {
        if let Some(window_entity) = target_window_entity {
            match cursor_options_query.get_mut(window_entity) {
                Ok(mut options) => {
                    if cursor_grab && window_focused {
                        options.grab_mode = CursorGrabMode::Locked;
                        options.visible = false;
                        debug!(controller = ?entity, window = ?window_entity, "Locked cursor to window");
                    } else {
                        options.grab_mode = CursorGrabMode::None;
                        options.visible = true;
                        debug!(controller = ?entity, window = ?window_entity, "Released cursor lock");
                    }
                }
                Err(_) => {
                    debug!(
                        controller = ?entity,
                        window = ?window_entity,
                        "Failed to access CursorOptions for target window"
                    );
                }
            }
        } else {
            debug!(controller = ?entity, "Camera target is not a window; skipping cursor updates");
        }
    }

    if !window_focused {
        return;
    }

    let focus_active = focus.map(|focus| focus.has_focus()).unwrap_or(false);
    if focus_active {
        return;
    }

    let mut scroll = 0.0;
    let amount = match accumulated_mouse_scroll.unit {
        MouseScrollUnit::Line => accumulated_mouse_scroll.delta.y,
        MouseScrollUnit::Pixel => accumulated_mouse_scroll.delta.y / 16.0,
    };
    scroll += amount;
    controller.walk_speed += scroll * controller.scroll_factor * controller.walk_speed;
    controller.run_speed = controller.walk_speed * 3.0;

    // Handle key input
    let mut axis_input = Vec3::ZERO;
    if key_input.pressed(controller.key_forward) {
        axis_input.z += 1.0;
    }
    if key_input.pressed(controller.key_back) {
        axis_input.z -= 1.0;
    }
    if key_input.pressed(controller.key_right) {
        axis_input.x += 1.0;
    }
    if key_input.pressed(controller.key_left) {
        axis_input.x -= 1.0;
    }
    if key_input.pressed(controller.key_up) {
        axis_input.y += 1.0;
    }
    if key_input.pressed(controller.key_down) {
        axis_input.y -= 1.0;
    }

    if axis_input != Vec3::ZERO {
        debug!(controller = ?entity, axis = ?axis_input, "Applying movement input");
        let max_speed = if key_input.pressed(controller.key_run) {
            controller.run_speed
        } else {
            controller.walk_speed
        };
        controller.velocity = axis_input.normalize() * max_speed;
    } else {
        let friction = controller.friction.clamp(0.0, 1.0);
        controller.velocity *= 1.0 - friction;
        if controller.velocity.length_squared() < 1e-6 {
            controller.velocity = Vec3::ZERO;
        }
    }

    // Apply movement update
    if controller.velocity != Vec3::ZERO {
        let forward = *transform.forward();
        let right = *transform.right();
        transform.translation += controller.velocity.x * dt * right
            + controller.velocity.y * dt * Vec3::Y
            + controller.velocity.z * dt * forward;
    }

    // Handle mouse input
    if accumulated_mouse_motion.delta != Vec2::ZERO && cursor_grab {
        // Apply look update
        controller.pitch = (controller.pitch
            - accumulated_mouse_motion.delta.y * RADIANS_PER_DOT * controller.sensitivity)
            .clamp(-PI / 2., PI / 2.);
        controller.yaw -=
            accumulated_mouse_motion.delta.x * RADIANS_PER_DOT * controller.sensitivity;
        transform.rotation = Quat::from_euler(EulerRot::ZYX, 0.0, controller.yaw, controller.pitch);
    }
}

fn focus_on_hovered_entity(
    keys: Res<ButtonInput<KeyCode>>,
    hovered: Res<HoveredEntity>,
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

    if rig.has_focus() {
        if let Some(mut controller) = camera_controller {
            controller.sync_from_transform(camera_transform);
        }
        rig.release_focus(camera_transform);
        return;
    }

    let Some(target_entity) = hovered.0 else {
        return;
    };

    let Ok(target_transform) = targets.get(target_entity) else {
        return;
    };

    rig.focus_on(
        target_entity,
        camera_transform,
        target_transform.translation(),
    );
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

fn drive_outline_materials(
    time: Res<Time>,
    camera: Query<&CameraFocusController>,
    mut materials: ResMut<Assets<GlowMaterial>>,
    targets: Query<(Entity, &MeshMaterial3d<GlowMaterial>), With<FocusTarget>>,
) {
    let Ok(rig) = camera.single() else {
        return;
    };

    let phase = time.elapsed_secs();
    for (entity, material_handle) in &targets {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            material.extension.set_phase(phase);
            let glow_strength = if Some(entity) == rig.focus { 1.0 } else { 0.0 };
            material.extension.set_glow_strength(glow_strength);
            material.extension.set_outline_width(0.45);
        }
    }
}

/// Stores the currently hovered entity when the pointer enters a [`FocusTarget`].
pub fn store_hover_on_enter(over: On<Pointer<Over>>, mut hovered: ResMut<HoveredEntity>) {
    hovered.0 = Some(over.entity);
}

/// Clears the hovered entity when the pointer exits the currently tracked [`FocusTarget`].
pub fn clear_hover_on_exit(out: On<Pointer<Out>>, mut hovered: ResMut<HoveredEntity>) {
    if hovered.0 == Some(out.entity) {
        hovered.0 = None;
    }
}

trait CameraControllerSyncExt {
    fn sync_from_transform(&mut self, transform: &Transform);
}

impl CameraControllerSyncExt for CameraController {
    fn sync_from_transform(&mut self, transform: &Transform) {
        let (yaw, pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        self.pitch = pitch;
    }
}
