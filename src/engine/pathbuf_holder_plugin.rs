use bevy::prelude::*;
use std::path::{Path, PathBuf};

pub struct PathBufHolderPlugin;

impl Plugin for PathBufHolderPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PathBufHolder>();
    }
}

#[derive(Component, Reflect, Debug, Deref, DerefMut, Default)]
pub struct PathBufHolder {
    pub path: Option<PathBuf>,
}
impl PathBufHolder {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }
    pub fn get<'a>(&'a self) -> eyre::Result<&'a PathBuf> {
        match self {
            PathBufHolder { path: Some(p) } => Ok(p),
            PathBufHolder { path: None } => Err(eyre::eyre!("PathBufHolder has no path set")),
        }
    }
    pub fn join(&self, other: impl AsRef<Path>) -> eyre::Result<PathBuf> {
        Ok(self.get()?.join(other))
    }
}
