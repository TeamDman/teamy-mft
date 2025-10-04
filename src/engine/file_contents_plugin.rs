use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use bytes::Bytes;
use std::io;
use std::path::PathBuf;

pub struct FileContentsPlugin;

impl Plugin for FileContentsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FileContents>();
        app.register_type::<RequestReadFileBytes>();
        app.register_type::<RequestWriteFileBytes>();
        app.add_systems(Update, (spawn_file_read_tasks, spawn_file_write_tasks));
        app.add_systems(Update, finish_file_contents_tasks);
    }
}

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct FileContents {
    #[reflect(ignore)]
    bytes: Bytes,
}

impl FileContents {
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }
}

#[derive(Component, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct RequestReadFileBytes;

#[derive(Component, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct RequestWriteFileBytes;

#[derive(Debug)]
enum FileContentsTask {
    Read(Task<Result<Bytes, io::Error>>),
    Write(Task<Result<(), io::Error>>),
}

#[derive(Component, Debug)]
pub struct FileContentsInProgress {
    task: FileContentsTask,
}

// #[derive(EntityEvent, Debug, Reflect)]
// pub struct FileContentsRead(Entity);

// #[derive(EntityEvent, Debug, Reflect)]
// pub struct FileContentsWritten(Entity);

impl FileContentsInProgress {
    fn new_read(task: Task<Result<Bytes, io::Error>>) -> Self {
        Self {
            task: FileContentsTask::Read(task),
        }
    }

    fn new_write(task: Task<Result<(), io::Error>>) -> Self {
        Self {
            task: FileContentsTask::Write(task),
        }
    }
}

fn spawn_file_read_tasks(
    mut commands: Commands,
    query: Query<
        (Entity, &PathBufHolder),
        (With<RequestReadFileBytes>, Without<FileContentsInProgress>),
    >,
) {
    for (entity, holder) in &query {
        let path: PathBuf = holder.to_path_buf();
        let pool = IoTaskPool::get();
        let read_path = path.clone();
        let task = pool.spawn(async move { std::fs::read(&read_path).map(Bytes::from) });
        debug!(?entity, ?path, "Spawning file read task");
        commands
            .entity(entity)
            .insert(FileContentsInProgress::new_read(task));
    }
}

fn spawn_file_write_tasks(
    mut commands: Commands,
    query: Query<
        (Entity, &PathBufHolder, &FileContents),
        (With<RequestWriteFileBytes>, Without<FileContentsInProgress>),
    >,
) {
    for (entity, holder, contents) in &query {
        let path: PathBuf = holder.to_path_buf();
        let pool = IoTaskPool::get();

        let bytes = contents.bytes().clone();
        let write_path = path.clone();
        let task = pool.spawn(async move {
            if let Some(parent) = write_path.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    return Err(error);
                }
            }
            std::fs::write(&write_path, bytes.as_ref())
        });

        debug!(?entity, ?path, "Spawning file write task");
        commands
            .entity(entity)
            .insert(FileContentsInProgress::new_write(task));
    }
}

fn finish_file_contents_tasks(
    mut commands: Commands,
    mut tasks: Query<(
        Entity,
        &mut FileContentsInProgress,
        Option<&RequestReadFileBytes>,
    )>,
    write_requests: Query<Entity, With<RequestWriteFileBytes>>,
) {
    for (entity, mut in_progress, has_read_request) in tasks.iter_mut() {
        match &mut in_progress.task {
            FileContentsTask::Read(task) => {
                let Some(result) = block_on(poll_once(task)) else {
                    continue;
                };

                let mut entity_commands = commands.entity(entity);
                entity_commands.remove::<FileContentsInProgress>();
                if has_read_request.is_some() {
                    entity_commands.remove::<RequestReadFileBytes>();
                }

                match result {
                    Ok(bytes) => {
                        debug!(?entity, len = bytes.len(), "Read file contents");
                        entity_commands.insert(FileContents::new(bytes));
                    }
                    Err(error) => {
                        warn!(?entity, ?error, "Failed to read file contents");
                        entity_commands.remove::<FileContents>();
                    }
                }
            }
            FileContentsTask::Write(task) => {
                let Some(result) = block_on(poll_once(task)) else {
                    continue;
                };

                let mut entity_commands = commands.entity(entity);
                entity_commands.remove::<FileContentsInProgress>();
                if write_requests.get(entity).is_ok() {
                    entity_commands.remove::<RequestWriteFileBytes>();
                }

                if let Err(error) = result {
                    warn!(?entity, ?error, "Failed to write file contents");
                } else {
                    debug!(?entity, "Wrote file contents");
                }
            }
        }
    }
}
