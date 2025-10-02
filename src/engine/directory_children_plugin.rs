use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct DirectoryChildrenPlugin;

impl Plugin for DirectoryChildrenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DirectoryChildrenTasks>();
        app.add_observer(queue_directory_listing_tasks);
        app.add_systems(Update, finish_directory_listing_tasks);
    }
}

/// A component that requests the children of a directory to be enumerated.
/// When added to an entity with a PathBufHolder, it will spawn an async task
/// that lists the directory contents and creates PathBufHolder entities for each child.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
pub struct RequestDirectoryChildren;

/// A debounce component indicating that a directory listing task is currently in progress
/// for this entity. Prevents duplicate tasks from being spawned.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
pub struct RequestDirectoryChildrenInFlight;

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
struct DirectoryChildrenTasks {
    /// Maps entity requesting directory listing to the task that will produce the list of child paths.
    #[reflect(ignore)]
    tasks: HashMap<Entity, Task<Result<Vec<PathBuf>, std::io::Error>>>,
}

/// Observer that responds to RequestDirectoryChildren being added.
/// Spawns async IO tasks to list directory contents.
fn queue_directory_listing_tasks(
    add: On<Add, RequestDirectoryChildren>,
    query: Query<
        (Entity, &PathBufHolder),
        (
            With<RequestDirectoryChildren>,
            Without<RequestDirectoryChildrenInFlight>,
        ),
    >,
    mut tasks: ResMut<DirectoryChildrenTasks>,
    mut commands: Commands,
) {
    let entity = add.entity;

    // Check if already in flight
    if tasks.tasks.contains_key(&entity) {
        warn!(
            ?entity,
            "RequestDirectoryChildren added but task already in flight; ignoring"
        );
        return;
    }

    // Get the path to list
    let (entity, path_holder) = match query.get(entity) {
        Ok(data) => data,
        Err(_) => {
            warn!(
                ?entity,
                "RequestDirectoryChildren added but entity missing PathBufHolder; ignoring"
            );
            return;
        }
    };

    let path = path_holder.to_path_buf();

    debug!(?entity, ?path, "Spawning directory listing task");

    // Spawn the async task
    let pool = IoTaskPool::get();
    let task = pool.spawn(async move {
        let entries = std::fs::read_dir(&path)?;
        let mut children = Vec::new();

        for entry_result in entries {
            match entry_result {
                Ok(entry) => {
                    children.push(entry.path());
                }
                Err(e) => {
                    warn!(?path, error=?e, "Failed to read directory entry");
                }
            }
        }

        Ok(children)
    });

    // Track the task
    tasks.tasks.insert(entity, task);

    // Add debounce component
    commands
        .entity(entity)
        .insert(RequestDirectoryChildrenInFlight);
}

/// System that polls completed tasks and spawns PathBufHolder entities
/// as children of the requesting entity.
fn finish_directory_listing_tasks(
    mut commands: Commands,
    mut tasks: ResMut<DirectoryChildrenTasks>,
) {
    let mut completed = Vec::new();

    for (entity, task) in tasks.tasks.iter_mut() {
        if let Some(result) = block_on(poll_once(task)) {
            match result {
                Ok(child_paths) => {
                    debug!(
                        ?entity,
                        count = child_paths.len(),
                        "Directory listing task completed successfully"
                    );

                    // Spawn child entities with PathBufHolder and ChildOf relationship
                    commands.entity(*entity).with_children(|parent| {
                        for child_path in child_paths {
                            parent.spawn((
                                PathBufHolder::new(child_path.clone()),
                                Name::new(format!(
                                    "Path: {}",
                                    child_path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("<invalid>")
                                )),
                            ));
                        }
                    });
                }
                Err(e) => {
                    warn!(
                        ?entity,
                        error=?e,
                        "Directory listing task failed"
                    );
                }
            }

            completed.push(*entity);
        }
    }

    // Clean up completed tasks
    for entity in completed {
        tasks.tasks.remove(&entity);
        commands
            .entity(entity)
            .remove::<RequestDirectoryChildren>()
            .remove::<RequestDirectoryChildrenInFlight>();
    }
}
