use crate::engine::camera_orbit_controller_plugin::CameraFocusController;
use bevy::camera::RenderTarget;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::input::mouse::MouseScrollUnit;
use bevy::prelude::*;
use bevy::window::WindowRef;
use std::f32::consts::*;
use std::fmt;

/// Handles WASD-style free-fly camera controls.
pub struct CameraWasdControllerPlugin;

impl Plugin for CameraWasdControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive_wasd_camera);
    }
}

/// Based on Valorant's default sensitivity.
pub const RADIANS_PER_DOT: f32 = 1.0 / 180.0;

/// Camera controller [`Component`].
#[derive(Component)]
pub struct CameraController {
    /// Enables this [`CameraController`] when `true`.
    pub enabled: bool,
    /// Indicates if this controller has been initialized by the [`CameraWasdControllerPlugin`].
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
    /// Tracks keyboard toggle-based cursor grabbing.
    pub cursor_grab_toggle: bool,
    /// Tracks mouse button-based cursor grabbing.
    pub cursor_grab_button: bool,
    /// Tracks whether the target window currently has focus.
    pub window_focused: bool,
}

impl CameraController {
    /// Returns `true` if the cursor is currently locked to the window.
    pub fn cursor_grabbed(&self) -> bool {
        self.cursor_grab_toggle || self.cursor_grab_button
    }
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
            cursor_grab_toggle: false,
            cursor_grab_button: false,
            window_focused: true,
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

fn drive_wasd_camera(
    time: Res<Time<Real>>,
    windows: Query<(Entity, &Window)>,
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    accumulated_mouse_scroll: Res<AccumulatedMouseScroll>,
    key_input: Res<ButtonInput<KeyCode>>,
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

    let target_window_entity = match camera.target.clone() {
        RenderTarget::Window(window_ref) => match window_ref {
            WindowRef::Entity(window_entity) => Some(window_entity),
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

    controller.window_focused = window_focused;

    if !window_focused {
        return;
    }

    let focus_active = focus.map(|focus| focus.has_focus()).unwrap_or(false);
    if focus_active {
        return;
    }

    let scroll = match accumulated_mouse_scroll.unit {
        MouseScrollUnit::Line => accumulated_mouse_scroll.delta.y,
        MouseScrollUnit::Pixel => accumulated_mouse_scroll.delta.y / 16.0,
    };
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
    if accumulated_mouse_motion.delta != Vec2::ZERO && controller.cursor_grabbed() {
        controller.pitch = (controller.pitch
            - accumulated_mouse_motion.delta.y * RADIANS_PER_DOT * controller.sensitivity)
            .clamp(-PI / 2., PI / 2.);
        controller.yaw -=
            accumulated_mouse_motion.delta.x * RADIANS_PER_DOT * controller.sensitivity;
        transform.rotation = Quat::from_euler(EulerRot::ZYX, 0.0, controller.yaw, controller.pitch);
    }
}

pub trait CameraControllerSyncExt {
    fn sync_from_transform(&mut self, transform: &Transform);
}

impl CameraControllerSyncExt for CameraController {
    fn sync_from_transform(&mut self, transform: &Transform) {
        let (yaw, pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        self.pitch = pitch;
    }
}
