use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use chrono::DateTime;
use chrono::Local;
use std::time::Instant;

pub struct FileMetadataPlugin;

impl Plugin for FileMetadataPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_request_file_metadata);
        app.add_systems(Update, finish_file_metadata_tasks);
    }
}

/// Request to fetch file metadata for an entity with a PathBufHolder.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct RequestFileMetadata;

/// Indicates a file metadata fetch is in progress.
#[derive(Component, Debug)]
pub struct RequestFileMetadataInProgress {
    pub task: Task<Result<std::fs::Metadata, std::io::Error>>,
}

// Outcome components from metadata

/// The instant when file metadata was observed.
#[derive(Component, Debug)]
pub struct FileMetadataObservedAt(
    pub Instant
);

/// The path exists on the filesystem.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct Exists;

/// The path does not exist on the filesystem.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct NotExists;

/// Last modified timestamp.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct LastModified(
    #[reflect(ignore)]
    pub DateTime<Local>
);

/// Created timestamp.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct CreatedAt(
    #[reflect(ignore)]
    pub DateTime<Local>
);

/// The path is a file.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct IsFile;

/// The path is a directory.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct IsDirectory;

/// The path is a symlink.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct IsSymlink;

/// File size in bytes.
#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct FileSize(pub u64);

/// File is read-only.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct IsReadOnly;

fn on_request_file_metadata(
    trigger: On<Add, RequestFileMetadata>,
    path_holders: Query<&PathBufHolder>,
    mut commands: Commands,
) {
    let entity = trigger.entity;
    
    let Ok(path_holder) = path_holders.get(entity) else {
        warn!(?entity, "RequestFileMetadata added to entity without PathBufHolder");
        commands.entity(entity).remove::<RequestFileMetadata>();
        return;
    };
    
    let path = path_holder.to_path_buf();
    debug!(?entity, ?path, "Spawning file metadata fetch task");
    
    let pool = IoTaskPool::get();
    let task = pool.spawn(async move {
        std::fs::metadata(&path)
    });
    
    commands.entity(entity)
        .insert(RequestFileMetadataInProgress { task });
}

fn finish_file_metadata_tasks(
    mut tasks: Query<(Entity, &mut RequestFileMetadataInProgress)>,
    mut commands: Commands,
) {
    for (entity, mut in_progress) in tasks.iter_mut() {
        let Some(result) = block_on(poll_once(&mut in_progress.task)) else {
            continue;
        };
        
        debug!(?entity, "File metadata task completed");
        
        // Insert observation timestamp
        commands.entity(entity).insert(FileMetadataObservedAt(Instant::now()));
        
        match result {
            Ok(metadata) => {
                // Path exists
                commands.entity(entity).insert(Exists);
                
                // File type
                if metadata.is_file() {
                    commands.entity(entity).insert(IsFile);
                }
                if metadata.is_dir() {
                    commands.entity(entity).insert(IsDirectory);
                }
                if metadata.is_symlink() {
                    commands.entity(entity).insert(IsSymlink);
                }
                
                // File size
                commands.entity(entity).insert(FileSize(metadata.len()));
                
                // Timestamps
                if let Ok(modified) = metadata.modified() {
                    let datetime: DateTime<Local> = modified.into();
                    commands.entity(entity).insert(LastModified(datetime));
                }
                
                if let Ok(created) = metadata.created() {
                    let datetime: DateTime<Local> = created.into();
                    commands.entity(entity).insert(CreatedAt(datetime));
                }
                
                // Permissions
                if metadata.permissions().readonly() {
                    commands.entity(entity).insert(IsReadOnly);
                }
                
                debug!(?entity, "Inserted file metadata components");
            }
            Err(error) => {
                // Path does not exist (or other error)
                debug!(?entity, ?error, "Path does not exist or metadata inaccessible");
                commands.entity(entity).insert(NotExists);
            }
        }
        
        // Remove request and in-progress markers
        commands.entity(entity)
            .remove::<RequestFileMetadata>()
            .remove::<RequestFileMetadataInProgress>();
    }
}
