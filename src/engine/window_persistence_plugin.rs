use crate::engine::persistence_plugin::{
    Persistable, PersistenceKey, PersistenceLoad, PersistenceLoaded, PersistencePlugin,
    PersistenceProperty,
};
use bevy::math::IVec2;
use bevy::prelude::*;
use bevy::window::{WindowPosition, WindowResolution};
use std::path::{Path, PathBuf};

/// Component that marks a window entity for persistence.
/// When added to an entity with a Window component, it will automatically set up persistence.
#[derive(Component, Reflect, Debug)]
#[reflect(Component)]
pub struct PersistWindowProperties {
    pub key: PathBuf,
    pub default_position: Option<WindowPosition>,
    pub default_resolution: Option<WindowResolution>,
}

impl PersistWindowProperties {
    pub fn new<P: AsRef<Path>>(key: P) -> Self {
        Self {
            key: key.as_ref().to_path_buf(),
            default_position: None,
            default_resolution: None,
        }
    }

    pub fn with_defaults(mut self, position: WindowPosition, resolution: WindowResolution) -> Self {
        self.default_position = Some(position);
        self.default_resolution = Some(resolution);
        self
    }
}

/// Shared persistence property for window position and resolution.
#[derive(Debug, Reflect, PartialEq, Clone)]
pub struct WindowData {
    pub position: WindowPosition,
    pub resolution: WindowResolution,
}

impl Persistable for WindowData {}

impl From<&Window> for WindowData {
    fn from(window: &Window) -> Self {
        Self {
            position: window.position,
            resolution: window.resolution.clone(),
        }
    }
}

/// Plugin that handles window persistence for entities marked with PersistWindowProperties.
pub struct WindowPersistencePlugin;

impl Plugin for WindowPersistencePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PersistWindowProperties>();
        app.add_plugins(PersistencePlugin::<WindowData>::default());
        app.add_observer(handle_add_persist_window_properties);
        app.add_systems(Update, handle_window_change);
        app.add_observer(handle_persistence_loaded);
    }
}

fn handle_add_persist_window_properties(
    trigger: On<Add, PersistWindowProperties>,
    mut commands: Commands,
    query: Query<&PersistWindowProperties>,
) {
    if let Ok(props) = query.get(trigger.entity) {
        commands.entity(trigger.entity).insert((
            PersistenceKey::<WindowData>::new(props.key.clone()),
            PersistenceLoad::<WindowData>::default(),
        ));
    }
}

fn handle_window_change(
    changed: Query<
        (Entity, &Window, Option<&PersistenceProperty<WindowData>>),
        (Changed<Window>, With<PersistWindowProperties>),
    >,
    mut commands: Commands,
) {
    for (entity, window, persistence) in changed.iter() {
        let new = WindowData::from(window);
        // Avoid writing minimized windows
        if matches!(new.position, WindowPosition::At(pos) if pos == IVec2::new(-32000, -32000)) {
            continue;
        }
        // Avoid change detection if nothing actually changed
        if let Some(old) = persistence
            && old.value == new
        {
            continue;
        }

        commands
            .entity(entity)
            .insert(new.into_persistence_property());
    }
}

fn handle_persistence_loaded(
    trigger: On<PersistenceLoaded<WindowData>>,
    mut windows: Query<(&mut Window, &PersistWindowProperties)>,
    mut commands: Commands,
) {
    if let Ok((mut window, props)) = windows.get_mut(trigger.entity) {
        let mut data = trigger.property.value.clone();
        // If loaded position is minimized, use default if available
        if matches!(data.position, WindowPosition::At(pos) if pos == IVec2::new(-32000, -32000)) {
            if let Some(default_pos) = &props.default_position {
                data.position = default_pos.clone();
            } else {
                // Skip applying minimized position by using existing value
                data.position = window.position;
            }
        }
        if data.resolution.width() * data.resolution.height() == 0.0 {
            if let Some(default_res) = &props.default_resolution {
                data.resolution = default_res.clone();
            } else {
                // Skip applying zero resolution by using existing value
                data.resolution = window.resolution.clone();
            }
        }
        info!(
            ?trigger,
            ?data,
            "Applying loaded persistence data to window"
        );
        window.position = data.position;
        window.resolution = data.resolution.clone();

        // Insert the property so it can be tracked for changes
        commands
            .entity(trigger.entity)
            .insert(data.into_persistence_property());
    }
}
