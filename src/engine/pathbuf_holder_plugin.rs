use std::path::PathBuf;

use bevy::prelude::*;

pub struct PathBufHolderPlugin;

impl Plugin for PathBufHolderPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PathBufHolder>();
    }
}

#[derive(Component, Reflect, Debug, Deref, DerefMut)]
pub struct PathBufHolder {
    pub path: PathBuf,
}
impl PathBufHolder {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}