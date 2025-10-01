use crate::engine::bytes_plugin::BytesHolder;
use crate::engine::bytes_plugin::BytesReceived;
use crate::engine::bytes_plugin::BytesReceiver;
use crate::engine::bytes_plugin::WriteBytesFromSourcesInProgress;
use crate::engine::bytes_plugin::WriteBytesToSink;
use crate::engine::bytes_plugin::WriteBytesToSinkInProgress;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use bytes::Bytes;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

pub struct FileBytesPlugin;

impl Plugin for FileBytesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(queue_file_write_tasks);
        app.add_observer(queue_file_read_tasks);
        app.add_systems(Update, finish_write_tasks);
        app.add_systems(Update, finish_read_tasks);
        app.init_resource::<FileBytesTasks>();
    }
}

#[derive(Default, Resource)]
struct FileBytesTasks {
    write_tasks_by_sink: HashMap<Entity, Task<()>>,
    read_tasks_by_receiver: HashMap<Entity, Task<Result<Bytes, std::io::Error>>>,
}

fn queue_file_write_tasks(
    add: On<Add, WriteBytesToSink>,
    sources: Query<(Entity, &BytesHolder, &WriteBytesToSink)>,
    sinks: Query<&PathBufHolder, Without<WriteBytesFromSourcesInProgress>>,
    mut commands: Commands,
    mut tasks: ResMut<FileBytesTasks>,
) {
    // Identify write requests
    let (source_entity, source_bytes, write_request) = match sources.get(add.entity) {
        Ok(x) => x,
        Err(_) => {
            // The write request is not targeting a BytesHolder source; ignore
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
    debug!(
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
                debug!(
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

fn queue_file_read_tasks(
    add: On<Add, WriteBytesToSink>,
    sources: Query<(Entity, &PathBufHolder, &WriteBytesToSink)>,
    receivers: Query<&BytesReceiver>,
    mut tasks: ResMut<FileBytesTasks>,
) {
    // Identify read requests (PathBufHolder -> BytesReceiver)
    let (source_entity, source_path_holder, write_request) = match sources.get(add.entity) {
        Ok(x) => x,
        Err(_) => return, // Not targeting a PathBufHolder source; ignore
    };

    // Check if sink is a BytesReceiver
    if receivers.get(write_request.0).is_err() {
        return; // Not a read request, let write handler deal with it
    }

    // Debounce: if a read is already in progress for this receiver, ignore this request
    if tasks.read_tasks_by_receiver.contains_key(&write_request.0) {
        return;
    }

    // Get source path
    let source_path: PathBuf = match &source_path_holder.path {
        Some(path) => path.clone(),
        None => {
            warn!(
                ?source_entity,
                ?write_request,
                "Source entity does not have a PathBufHolder with a path; cannot read bytes"
            );
            return;
        }
    };

    let receiver_entity = write_request.0;

    // Log
    debug!(
        ?source_entity,
        ?receiver_entity,
        ?source_path,
        "Spawning read-bytes task"
    );

    // Acquire task pool
    let pool = IoTaskPool::get();

    // Spawn task
    let task = pool.spawn(async move {
        let bytes = std::fs::read(&source_path)?;
        debug!(
            ?source_entity,
            ?receiver_entity,
            ?source_path,
            "Read {} bytes from file",
            bytes.len()
        );
        Ok(Bytes::from(bytes))
    });

    // Track task
    tasks.read_tasks_by_receiver.insert(receiver_entity, task);
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

fn finish_read_tasks(mut commands: Commands, mut tasks: ResMut<FileBytesTasks>) {
    let mut completed = Vec::new();

    for (receiver_entity, pending) in tasks.read_tasks_by_receiver.iter_mut() {
        if let Some(result) = block_on(poll_once(pending)) {
            match result {
                Ok(bytes) => {
                    debug!(?receiver_entity, "Read task completed successfully");
                    commands
                        .entity(*receiver_entity)
                        .remove::<BytesReceiver>()
                        .insert(BytesHolder { bytes });
                    commands.trigger(BytesReceived {
                        entity: *receiver_entity,
                    });
                }
                Err(error) => {
                    warn!(?receiver_entity, ?error, "Read task failed");
                    commands.entity(*receiver_entity).remove::<BytesReceiver>();
                }
            }
            completed.push(*receiver_entity);
        }
    }

    for receiver_entity in completed {
        tasks.read_tasks_by_receiver.remove(&receiver_entity);
    }
}
