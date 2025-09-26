use crate::sync_dir::try_get_sync_dir;
use bevy::ecs::prelude::ReflectResource;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use std::ops::Deref;
use std::path::PathBuf;

pub struct SyncDirectoryPlugin;
impl Plugin for SyncDirectoryPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SyncDirectory>();
        app.add_systems(Startup, begin_load_sync_dir_from_preferences);
    }
}

#[derive(Resource, Reflect, Default, Debug)]
#[reflect(Resource)]
pub struct SyncDirectory(pub PathBuf);

impl Deref for SyncDirectory {
    type Target = PathBuf;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Resource, Default)]
pub struct SyncDirectoryTasks {
    get_sync_dir: Option<Task<Result<SyncDirectory>>>,
}

pub fn begin_load_sync_dir_from_preferences(mut tasks: ResMut<SyncDirectoryTasks>) -> Result<()> {
    let task_pool = IoTaskPool::get();
    let task = task_pool.spawn(async move {
        let path = try_get_sync_dir()?;
        Ok(SyncDirectory(path))
    });
    debug!(task=?task, "Spawned task to load sync dir from preferences");
    tasks.get_sync_dir = Some(task);
    Ok(())
}
pub fn finish_load_sync_dir_from_preferences(
    mut commands: Commands,
    mut tasks: ResMut<SyncDirectoryTasks>,
) -> Result<()> {
    if let Some(task) = tasks.get_sync_dir.as_mut() {
        if let Some(result) = block_on(poll_once(task)) {
            let sync_dir = result?;
            debug!(sync_dir=?sync_dir, "Loaded sync dir from preferences");
            commands.insert_resource(sync_dir);
            tasks.get_sync_dir = None;
        }
    }
    Ok(())
}
