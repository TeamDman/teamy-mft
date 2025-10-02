use bevy::prelude::*;
use std::path::PathBuf;

pub struct PathBufHolderPlugin;

impl Plugin for PathBufHolderPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PathBufHolder>();
    }
}

#[derive(Component, Reflect, Debug, Deref, DerefMut)]
pub struct PathBufHolder {
    path: PathBuf,
}
impl Default for PathBufHolder {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
        }
    }
}
impl PathBufHolder {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
        }
    }
}
