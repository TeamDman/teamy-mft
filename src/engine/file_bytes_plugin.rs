use crate::engine::bytes_plugin::ByteSource;
use crate::engine::bytes_plugin::WriteBytesFromSourcesInProgress;
use crate::engine::bytes_plugin::WriteBytesToSink;
use crate::engine::bytes_plugin::WriteBytesToSinkInProgress;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;

pub struct FileBytesPlugin;

impl Plugin for FileBytesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(queue_file_write_tasks);
        app.add_systems(Update, finish_write_tasks);
        app.init_resource::<FileBytesTasks>();
    }
}

#[derive(Default, Resource)]
struct FileBytesTasks {
    write_tasks_by_sink: HashMap<Entity, Task<()>>,
}

fn queue_file_write_tasks(
    add: On<Add, WriteBytesToSink>,
    sources: Query<(Entity, &ByteSource, &WriteBytesToSink)>,
    sinks: Query<&PathBufHolder, Without<WriteBytesFromSourcesInProgress>>,
    mut commands: Commands,
    mut tasks: ResMut<FileBytesTasks>,
) {
    // Identify write requests
    let (source_entity, source_bytes, write_request) = match sources.get(add.entity) {
        Ok(x) => x,
        Err(error) => {
            warn!(
                ?add.entity,
                ?error,
                "Failed to get WriteBytesToSink from added entity"
            );
            return;
        }
    };

    // Debounce: if a write is already in progress for this source, ignore this request
    if tasks.write_tasks_by_sink.contains_key(&source_entity) {
        return;
    }

    // Get sink path
    let sink_path: PathBuf = match sinks.get(write_request.0) {
        Ok(PathBufHolder { path: None }) => {
            warn!(
                ?source_entity,
                ?write_request,
                "Sink entity does not have a PathBufHolder with a path; cannot write bytes"
            );
            return;
        }
        Ok(PathBufHolder { path: Some(path) }) => path.clone(),
        Err(error) => {
            warn!(
                ?source_entity,
                ?write_request,
                ?error,
                "Failed to get PathBufHolder from sink entity"
            );
            return;
        }
    };

    // Clone bytes for task
    let bytes = source_bytes.bytes.clone();
    let source_entity = source_entity;
    let sink_entity = write_request.0;

    // Log
    info!(
        ?source_entity,
        ?sink_entity,
        ?sink_path,
        "Spawning write-bytes task"
    );

    // Acquire task pool
    let pool = IoTaskPool::get();

    // Spawn task
    let task = pool.spawn(async move {
        match std::fs::write(&sink_path, bytes.as_ref()) {
            Ok(()) => {
                info!(
                    ?source_entity,
                    ?sink_entity,
                    ?sink_path,
                    "Wrote {} bytes to file",
                    bytes.len()
                );
            }
            Err(error) => {
                warn!(
                    ?source_entity,
                    ?sink_entity,
                    ?sink_path,
                    ?error,
                    "Failed to write bytes"
                );
            }
        }
    });

    // Track task
    tasks.write_tasks_by_sink.insert(source_entity, task);

    // Add debounce
    commands
        .entity(source_entity)
        .insert(WriteBytesToSinkInProgress(sink_entity));
}

fn finish_write_tasks(mut commands: Commands, mut tasks: ResMut<FileBytesTasks>) {
    let mut completed = Vec::new();

    for (source_entity, pending) in tasks.write_tasks_by_sink.iter_mut() {
        if let Some(()) = block_on(poll_once(pending)) {
            commands.entity(*source_entity).remove::<WriteBytesToSink>();
            commands
                .entity(*source_entity)
                .remove::<WriteBytesToSinkInProgress>();
            completed.push(*source_entity);
        }
    }

    for source_entity in completed {
        tasks.write_tasks_by_sink.remove(&source_entity);
    }
}
