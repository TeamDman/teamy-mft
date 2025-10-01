use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use bytes::Bytes;
use eyre::Context;
use eyre::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;

pub struct WriteFileContentPlugin;

impl Plugin for WriteFileContentPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ByteSource>();
        app.register_type::<PathBufHolder>();
        app.register_type::<WriteBytesToSink>();
        app.register_type::<WriteBytesFromSources>();
        app.init_resource::<WriteBytesTasks>();
        app.add_systems(Update, queue_file_write_tasks);
        app.add_systems(Update, finish_write_tasks);
    }
}

#[derive(Component, Reflect, Debug)]
pub struct ByteSource {
    #[reflect(ignore)]
    pub bytes: Bytes,
}

#[derive(Component, Reflect, Debug)]
#[relationship(relationship_target = WriteBytesFromSources)]
pub struct WriteBytesToSink(pub Entity);

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
#[relationship_target(relationship = WriteBytesToSink, linked_spawn)]
pub struct WriteBytesFromSources(Vec<Entity>);

impl WriteBytesFromSources {
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.0.iter().copied()
    }
}

#[derive(Component, Reflect, Debug)]
pub struct PathBufHolder {
    pub path: PathBuf,
}

#[derive(Default, Resource)]
struct WriteBytesTasks {
    active: HashMap<Entity, PendingWriteTask>,
}

struct PendingWriteTask {
    sink: Entity,
    task: Task<Result<()>>,
}

fn queue_file_write_tasks(
    mut commands: Commands,
    mut tasks: ResMut<WriteBytesTasks>,
    sinks: Query<(Entity, &WriteBytesFromSources, &PathBufHolder)>,
    sources: Query<&ByteSource>,
) {
    let pool = IoTaskPool::get();

    for (sink_entity, write_requests, path) in sinks.iter() {
        debug!(
            ?sink_entity,
            ?write_requests,
            "Processing write tasks for sink entity"
        );

        for source_entity in write_requests.iter() {
            if tasks.active.contains_key(&source_entity) {
                continue;
            }

            let source_bytes = match sources.get(source_entity) {
                Ok(bytes) => bytes,
                Err(error) => {
                    warn!(
                        ?source_entity,
                        ?error,
                        "Failed to get BytesHolder from source entity"
                    );
                    continue;
                }
            };

            let sink_path = path.path.clone();
            let bytes = source_bytes.bytes.clone();
            let source_id = source_entity;
            let sink_id = sink_entity;

            commands.entity(sink_entity).insert(ByteSource {
                bytes: bytes.clone(),
            });

            let task = pool.spawn(async move {
                std::fs::write(&sink_path, bytes.as_ref()).wrap_err_with(|| {
                    format!(
                        "Failed to write bytes from source entity {source_id:?} to sink entity {sink_id:?} at {}",
                        sink_path.display()
                    )
                })?;
                Ok(())
            });

            info!(?source_entity, ?sink_entity, path=%path.path.display(), "Spawned write-bytes task");

            tasks.active.insert(
                source_entity,
                PendingWriteTask {
                    sink: sink_entity,
                    task,
                },
            );
        }
    }
}

fn finish_write_tasks(mut commands: Commands, mut tasks: ResMut<WriteBytesTasks>) {
    let mut completed = Vec::new();

    for (source_entity, pending) in tasks.active.iter_mut() {
        if let Some(result) = block_on(poll_once(&mut pending.task)) {
            match result {
                Ok(()) => {
                    info!(?source_entity, sink=?pending.sink, "Completed write-bytes task");
                    commands.entity(*source_entity).remove::<WriteBytesToSink>();
                    completed.push(*source_entity);
                }
                Err(error) => {
                    warn!(?source_entity, sink=?pending.sink, ?error, "Write-bytes task failed");
                    commands.entity(*source_entity).remove::<WriteBytesToSink>();
                    completed.push(*source_entity);
                }
            }
        }
    }

    for source_entity in completed {
        tasks.active.remove(&source_entity);
    }
}
