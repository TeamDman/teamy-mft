use crate::engine::file_contents_plugin::FileContents;
use crate::engine::file_contents_plugin::FileContentsInProgress;
use crate::engine::file_contents_plugin::RequestReadFileBytes;
use crate::engine::file_metadata_plugin::Exists;
use crate::engine::file_metadata_plugin::IsFile;
use crate::engine::file_metadata_plugin::NotExists;
use crate::engine::file_metadata_plugin::RequestFileMetadata;
use crate::engine::file_metadata_plugin::RequestFileMetadataInProgress;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::sync_dir_plugin::SyncDirectory;
use crate::mft::mft_file::MftFile;
use bevy::ecs::relationship::Relationship;
use bevy::prelude::*;
use bytes::BytesMut;
use std::path::Path;

pub struct MftFilePlugin;

impl Plugin for MftFilePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<LoadCachedMftFilesGoal>();
        app.register_type::<IsMftFile>();
        app.register_type::<IsNotMftFile>();
        app.init_resource::<LoadCachedMftFilesGoal>();
        app.add_systems(
            Update,
            (
                request_metadata_for_sync_dir_children,
                mark_mft_files,
                queue_mft_file_reads,
                load_mft_files_from_contents,
            )
                .run_if(|goal: Res<LoadCachedMftFilesGoal>| goal.enabled),
        );
    }
}

#[derive(Resource, Reflect, Debug, Clone)]
#[reflect(Resource)]
pub struct LoadCachedMftFilesGoal {
    pub enabled: bool,
}

impl Default for LoadCachedMftFilesGoal {
    fn default() -> Self {
        Self { enabled: false }
    }
}

/// Marker component indicating this PathBufHolder entity represents an on-disk `.mft` file.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct IsMftFile;

/// Marker component indicating the entity is known not to be an `.mft` file.
#[derive(Component, Debug, Reflect, Default)]
#[reflect(Component)]
pub struct IsNotMftFile;

pub fn request_metadata_for_sync_dir_children(
    mut commands: Commands,
    sync_dirs: Query<(), With<SyncDirectory>>,
    candidates: Query<
        (Entity, &ChildOf, &PathBufHolder),
        (
            Without<RequestFileMetadata>,
            Without<RequestFileMetadataInProgress>,
            Without<Exists>,
            Without<NotExists>,
        ),
    >,
) {
    for (entity, child_of, holder) in &candidates {
        let parent_entity = child_of.get();
        if sync_dirs.get(parent_entity).is_err() {
            continue;
        }

        debug!(
            ?entity,
            path = ?holder.as_path(),
            "Requesting metadata for SyncDirectory child"
        );
        commands.entity(entity).insert(RequestFileMetadata);
    }
}

pub fn mark_mft_files(
    mut commands: Commands,
    sync_dirs: Query<(), With<SyncDirectory>>,
    candidates: Query<
        (Entity, &ChildOf, &PathBufHolder),
        (
            With<Exists>,
            With<IsFile>,
            Without<IsMftFile>,
            Without<IsNotMftFile>,
        ),
    >,
) {
    for (entity, child_of, holder) in &candidates {
        let parent_entity = child_of.get();
        if sync_dirs.get(parent_entity).is_err() {
            continue;
        }

        if is_mft_path(holder.as_path()) {
            debug!(?entity, path = ?holder.as_path(), "Identified .mft file");
            commands
                .entity(entity)
                .insert(IsMftFile)
                .remove::<IsNotMftFile>();
        } else {
            commands
                .entity(entity)
                .insert(IsNotMftFile)
                .remove::<IsMftFile>();
        }
    }
}

pub fn queue_mft_file_reads(
    mut commands: Commands,
    candidates: Query<
        Entity,
        (
            With<IsMftFile>,
            With<PathBufHolder>,
            With<Exists>,
            With<IsFile>,
            Without<MftFile>,
            Without<FileContents>,
            Without<FileContentsInProgress>,
            Without<RequestReadFileBytes>,
        ),
    >,
) {
    for entity in &candidates {
        debug!(?entity, "Queueing read bytes request for MFT file");
        commands.entity(entity).insert(RequestReadFileBytes);
    }
}

fn load_mft_files_from_contents(
    mut commands: Commands,
    mut query: Query<
        (Entity, &mut FileContents),
        (With<IsMftFile>, Added<FileContents>, Without<MftFile>),
    >,
) {
    for (entity, mut contents) in query.iter_mut() {
        commands.entity(entity).remove::<FileContents>();
        let bytes = contents.take_bytes();
        if !bytes.is_unique() {
            warn!(
                ?entity,
                len = bytes.len(),
                "Converting shared Bytes to BytesMut will clone, this MftFile construction may be slow"
            );
        }

        let mut_bytes = BytesMut::from(bytes);

        match MftFile::from_bytes(mut_bytes) {
            Ok(mft) => {
                info!(?entity, "Constructed MFT from cached bytes");
                commands.entity(entity).insert(mft);
            }
            Err(error) => {
                warn!(?entity, ?error, "Failed to construct MFT from bytes");
            }
        }
    }
}

fn is_mft_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("mft"))
        .unwrap_or(false)
}
