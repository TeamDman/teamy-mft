use crate::engine::directory_children_plugin::RequestDirectoryChildren;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::persistence_plugin::PersistenceDirectory;
use crate::sync_dir::try_get_sync_dir;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use std::any::type_name;
use std::path::PathBuf;

pub struct SyncDirectoryPlugin;
impl Plugin for SyncDirectoryPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SyncDirectory>();
        app.register_type::<SyncDirectoryEvent>();
        app.init_resource::<SyncDirectoryTasks>();
        app.add_observer(read_sync_directory_events_and_launch_task);
        app.add_systems(Update, finish_load_sync_dir_from_preferences);
        app.add_systems(Startup, begin_load_sync_dir_from_preferences);
    }
}

#[derive(Component, Reflect, Default, Debug)]
pub struct SyncDirectory;

#[derive(Event, Reflect, Clone, Copy, Debug)]
#[reflect]
pub enum SyncDirectoryEvent {
    ReadSyncDirectory,
}

#[derive(Resource, Default)]
pub struct SyncDirectoryTasks {
    get_sync_dir: Option<Task<Result<PathBuf>>>,
}

pub fn begin_load_sync_dir_from_preferences(mut commands: Commands) -> Result<()> {
    commands.trigger(SyncDirectoryEvent::ReadSyncDirectory);
    debug!("Emitted ReadSyncDirectory event");
    Ok(())
}

pub fn read_sync_directory_events_and_launch_task(
    event: On<SyncDirectoryEvent>,
    mut tasks: ResMut<SyncDirectoryTasks>,
) -> Result<()> {
    debug!(?event, "Processing {}", type_name::<SyncDirectoryEvent>());
    match *event {
        SyncDirectoryEvent::ReadSyncDirectory => {
            if tasks.get_sync_dir.is_some() {
                warn!(
                    "ReadSyncDirectory requested but a get_sync_dir task is already running; ignoring"
                );
                return Ok(());
            }

            let task_pool = IoTaskPool::get();
            let task = task_pool.spawn(async move {
                let path = try_get_sync_dir()?;
                Ok(path)
            });
            debug!(task=?task, "Spawned task to load sync dir from preferences");
            tasks.get_sync_dir = Some(task);
        }
    }

    Ok(())
}

pub fn finish_load_sync_dir_from_preferences(
    mut commands: Commands,
    mut tasks: ResMut<SyncDirectoryTasks>,
) -> Result<()> {
    if let Some(task) = tasks.get_sync_dir.as_mut() {
        if let Some(result) = block_on(poll_once(task)) {
            let sync_dir = result?;
            info!(sync_dir=?sync_dir, "Loaded sync dir from preferences");
            commands.spawn((
                Name::new(format!("SyncDirectory ({})", sync_dir.to_string_lossy())),
                SyncDirectory,
                PathBufHolder::new(sync_dir),
                PersistenceDirectory,
                RequestDirectoryChildren,
            ));
            tasks.get_sync_dir = None;
        }
    }
    Ok(())
}
