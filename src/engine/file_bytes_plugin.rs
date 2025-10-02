use crate::engine::bytes_plugin::BytesHolder;
use crate::engine::bytes_plugin::BytesReceived;
use crate::engine::bytes_plugin::BytesReceiver;
use crate::engine::bytes_plugin::BytesSent;
use crate::engine::bytes_plugin::WriteBytesFromSourcesInProgress;
use crate::engine::bytes_plugin::WriteBytesToSinkFinished;
use crate::engine::bytes_plugin::WriteBytesToSinkInProgress;
use crate::engine::bytes_plugin::WriteBytesToSinkRequested;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::paths::EnsureParentDirExists;
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
        app.add_systems(Update, finish_tasks);
        app.init_resource::<FileBytesTasks>();
    }
}

enum FileTask {
    /// Writing bytes from a BytesHolder source to a file sink.
    Write {
        /// The entity containing the BytesHolder being written.
        source: Entity,
        /// The entity containing the PathBufHolder file sink.
        sink: Entity,
        /// The async task performing the write operation.
        task: Task<()>,
    },
    /// Reading bytes from a file source to a BytesReceiver sink.
    Read {
        /// The entity containing the PathBufHolder file source.
        source: Entity,
        /// The entity containing the BytesReceiver sink.
        sink: Entity,
        /// The async task performing the read operation.
        task: Task<Result<Bytes, std::io::Error>>,
    },
}

#[derive(Default, Resource)]
struct FileBytesTasks {
    tasks_by_source: HashMap<Entity, FileTask>,
}

fn queue_file_write_tasks(
    add: On<Add, WriteBytesToSinkRequested>,
    sources: Query<(Entity, &BytesHolder, &WriteBytesToSinkRequested)>,
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
    if tasks.tasks_by_source.contains_key(&source_entity) {
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
        if let Err(e) = sink_path.ensure_parent_dir_exists() {
            warn!(
                ?source_entity,
                ?sink_entity,
                ?sink_path,
                ?e,
                "Failed to ensure parent directory exists; cannot write bytes"
            );
            return;
        }
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
    tasks.tasks_by_source.insert(
        source_entity,
        FileTask::Write {
            source: source_entity,
            sink: sink_entity,
            task,
        },
    );

    // Add debounce
    commands
        .entity(source_entity)
        .insert(WriteBytesToSinkInProgress(sink_entity));
}

fn queue_file_read_tasks(
    add: On<Add, WriteBytesToSinkRequested>,
    sources: Query<(Entity, &PathBufHolder, &WriteBytesToSinkRequested)>,
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

    // Debounce: if a read is already in progress for this source, ignore this request
    if tasks.tasks_by_source.contains_key(&source_entity) {
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

    // Spawn task that reads bytes and writes them to the receiver
    let task = pool.spawn(async move {
        match std::fs::read(&source_path) {
            Ok(bytes) => {
                debug!(
                    ?source_entity,
                    ?receiver_entity,
                    ?source_path,
                    "Read {} bytes from file",
                    bytes.len()
                );
                Ok(bytes::Bytes::from(bytes))
            }
            Err(error) => {
                warn!(
                    ?source_entity,
                    ?receiver_entity,
                    ?source_path,
                    ?error,
                    "Failed to read bytes"
                );
                Err(error)
            }
        }
    });

    // Track task with receiver entity
    tasks.tasks_by_source.insert(
        source_entity,
        FileTask::Read {
            source: source_entity,
            sink: receiver_entity,
            task,
        },
    );
}

fn finish_tasks(mut commands: Commands, mut tasks: ResMut<FileBytesTasks>) {
    let mut completed = Vec::new();

    for (source_entity, task) in tasks.tasks_by_source.iter_mut() {
        let outcome = match task {
            FileTask::Write {
                source,
                sink,
                task: pending,
            } => {
                if let Some(()) = block_on(poll_once(pending)) {
                    Some((source, sink))
                } else {
                    None
                }
            }
            FileTask::Read {
                source,
                sink,
                task: pending,
            } => {
                if let Some(result) = block_on(poll_once(pending)) {
                    match result {
                        Ok(bytes) => {
                            commands.entity(*sink).insert(BytesHolder { bytes });
                        }
                        Err(error) => {
                            warn!(?sink, ?error, "Read task failed");
                        }
                    }
                    commands.entity(*sink).remove::<BytesReceiver>();
                    Some((source, sink))
                } else {
                    None
                }
            }
        };
        let Some((source, sink)) = outcome else {
            continue;
        };

        assert_eq!(*source_entity, *source);
        debug!(?source, ?sink, "File task completed");

        // Trigger BytesSent event
        debug!(?source, ?sink, "Triggering BytesSent event on {source}");
        commands.trigger(BytesSent { entity: *source });

        // Trigger BytesReceived event
        debug!(?source, ?sink, "Triggering BytesReceived event on {sink}");
        commands.trigger(BytesReceived { entity: *sink });

        commands
            .entity(*source)
            .remove::<WriteBytesToSinkRequested>()
            .remove::<WriteBytesToSinkInProgress>()
            .insert(WriteBytesToSinkFinished(*sink));
        completed.push(*source_entity);
    }

    for source_entity in completed {
        tasks.tasks_by_source.remove(&source_entity);
    }
}
