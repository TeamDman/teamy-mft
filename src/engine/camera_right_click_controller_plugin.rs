use crate::engine::camera_wasd_controller_plugin::CameraController;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::window::CursorGrabMode;
use bevy::window::CursorOptions;
use bevy::window::WindowRef;

/// Handles right-click based cursor locking for the camera controller.
pub struct CameraRightClickControllerPlugin;

impl Plugin for CameraRightClickControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_camera_cursor_grab);
    }
}

fn update_camera_cursor_grab(
    windows: Query<(Entity, &Window)>,
    mut cursor_options_query: Query<&mut CursorOptions>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    key_input: Res<ButtonInput<KeyCode>>,
    mut controllers: Query<(Entity, &mut CameraController, &Camera)>,
) {
    for (entity, mut controller, camera) in &mut controllers {
        if !controller.enabled {
            continue;
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
            controller.cursor_grab_toggle = !controller.cursor_grab_toggle;
            cursor_grab_change = true;
            debug!(controller = ?entity, toggle = controller.cursor_grab_toggle, "Toggled cursor grab via keyboard");
        }

        let pressed_now = mouse_button_input.pressed(controller.mouse_key_cursor_grab);
        if pressed_now != controller.cursor_grab_button {
            controller.cursor_grab_button = pressed_now;
            cursor_grab_change = true;
            debug!(
                controller = ?entity,
                button = ?controller.mouse_key_cursor_grab,
                pressed = pressed_now,
                "Mouse grab button state changed"
            );
        }

        if !window_focused {
            if controller.cursor_grab_toggle || controller.cursor_grab_button {
                debug!(controller = ?entity, "Target window lost focus; resetting grab state");
            }
            if controller.cursor_grab_toggle {
                controller.cursor_grab_toggle = false;
                cursor_grab_change = true;
            }
            if controller.cursor_grab_button {
                controller.cursor_grab_button = false;
                cursor_grab_change = true;
            }
        }

        if cursor_grab_change {
            if let Some(window_entity) = target_window_entity {
                match cursor_options_query.get_mut(window_entity) {
                    Ok(mut options) => {
                        if controller.cursor_grabbed() && window_focused {
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

        controller.window_focused = window_focused;
    }
}
