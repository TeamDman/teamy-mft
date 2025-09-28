use crate::sync_dir::try_get_sync_dir;
use bevy::ecs::prelude::ReflectComponent;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use std::any::type_name;
use std::ops::Deref;
use std::path::PathBuf;

pub struct SyncDirectoryPlugin;
impl Plugin for SyncDirectoryPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SyncDirectory>();
        app.register_type::<SyncDirectoryEvents>();
        app.init_resource::<SyncDirectoryTasks>();
        app.add_message::<SyncDirectoryEvents>();
        app.add_systems(Startup, begin_load_sync_dir_from_preferences);
        app.add_systems(
            Update,
            (
                read_sync_directory_events_and_launch_task,
                finish_load_sync_dir_from_preferences,
            ),
        );
    }
}

#[derive(Component, Reflect, Default, Debug)]
#[reflect(Component)]
pub struct SyncDirectory(pub PathBuf);

impl Deref for SyncDirectory {
    type Target = PathBuf;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Message, Reflect, Clone, Copy, Debug)]
#[reflect]
pub enum SyncDirectoryEvents {
    ReadSyncDirectory,
}

#[derive(Resource, Default)]
pub struct SyncDirectoryTasks {
    get_sync_dir: Option<Task<Result<SyncDirectory>>>,
}

pub fn begin_load_sync_dir_from_preferences(
    mut messages: ResMut<Messages<SyncDirectoryEvents>>,
) -> Result<()> {
    messages.write(SyncDirectoryEvents::ReadSyncDirectory);
    debug!("Emitted ReadSyncDirectory event on startup");
    Ok(())
}

pub fn read_sync_directory_events_and_launch_task(
    mut events: MessageReader<SyncDirectoryEvents>,
    mut tasks: ResMut<SyncDirectoryTasks>,
) -> Result<()> {
    for ev in events.read() {
        info!(event=?ev, "Processing {}", type_name::<SyncDirectoryEvents>());
        match ev {
            SyncDirectoryEvents::ReadSyncDirectory => {
                if tasks.get_sync_dir.is_some() {
                    warn!(
                        "ReadSyncDirectory requested but a get_sync_dir task is already running; ignoring"
                    );
                    continue;
                }

                let task_pool = IoTaskPool::get();
                let task = task_pool.spawn(async move {
                    let path = try_get_sync_dir()?;
                    Ok(SyncDirectory(path))
                });
                info!(task=?task, "Spawned task to load sync dir from preferences");
                tasks.get_sync_dir = Some(task);
            }
        }
    }
    Ok(())
}

pub fn finish_load_sync_dir_from_preferences(
    mut commands: Commands,
    mut tasks: ResMut<SyncDirectoryTasks>,
    existing: Query<Entity, With<SyncDirectory>>,
) -> Result<()> {
    if let Some(task) = tasks.get_sync_dir.as_mut() {
        if let Some(result) = block_on(poll_once(task)) {
            let sync_dir = result?;
            info!(sync_dir=?sync_dir, "Loaded sync dir from preferences");
            if let Ok(entity) = existing.single() {
                commands.entity(entity).insert(sync_dir);
            } else {
                commands.spawn(sync_dir);
            }
            tasks.get_sync_dir = None;
        }
    }
    Ok(())
}
