//! Aggregates the individual camera controller plugins for convenience.

pub use crate::engine::camera_orbit_controller_plugin::CameraFocusController;
pub use crate::engine::camera_orbit_controller_plugin::CameraOrbitControllerPlugin;
pub use crate::engine::camera_orbit_controller_plugin::FocusTarget;
pub use crate::engine::camera_right_click_controller_plugin::CameraRightClickControllerPlugin;
pub use crate::engine::camera_wasd_controller_plugin::CameraController;
pub use crate::engine::camera_wasd_controller_plugin::CameraControllerSyncExt;
pub use crate::engine::camera_wasd_controller_plugin::CameraWasdControllerPlugin;
pub use crate::engine::camera_wasd_controller_plugin::RADIANS_PER_DOT;
use bevy::prelude::*;

/// A convenience plugin that wires together the WASD, right-click, and orbit camera plugins.
pub struct CameraControllerPlugin;

impl Plugin for CameraControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            CameraRightClickControllerPlugin,
            CameraWasdControllerPlugin,
            CameraOrbitControllerPlugin,
        ));
    }
}
