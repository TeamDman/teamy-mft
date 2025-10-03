use crate::engine::bytes_plugin::BytesHolder;
use crate::engine::bytes_plugin::BytesReceived;
use crate::engine::bytes_plugin::BytesReceiver;
use crate::engine::bytes_plugin::WriteBytesToSinkRequested;
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
use std::path::Path;

pub struct MftFilePlugin;

impl Plugin for MftFilePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<LoadCachedMftFilesGoal>();
        app.register_type::<IsMftFile>();
        app.register_type::<IsNotMftFile>();
        app.register_type::<MftFileBytesSink>();
        app.init_resource::<LoadCachedMftFilesGoal>();
        app.add_systems(
            Update,
            (
                request_metadata_for_sync_dir_children,
                mark_mft_files,
                queue_mft_file_reads,
            ),
        );
        app.add_observer(load_mft_file_on_bytes_received);
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

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
struct MftFileBytesSink {
    source: Entity,
}

pub fn request_metadata_for_sync_dir_children(
    goal: Res<LoadCachedMftFilesGoal>,
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
    if !goal.enabled {
        return;
    }

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
    goal: Res<LoadCachedMftFilesGoal>,
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
    if !goal.enabled {
        return;
    }

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
    goal: Res<LoadCachedMftFilesGoal>,
    mut commands: Commands,
    candidates: Query<
        Entity,
        (
            With<IsMftFile>,
            With<PathBufHolder>,
            With<Exists>,
            With<IsFile>,
            Without<BytesHolder>,
            Without<BytesReceiver>,
            Without<WriteBytesToSinkRequested>,
        ),
    >,
) {
    if !goal.enabled {
        return;
    }

    for entity in &candidates {
        debug!(?entity, "Queueing read bytes request for MFT file");
        let sink = commands
            .spawn((BytesReceiver, MftFileBytesSink { source: entity }))
            .id();
        commands
            .entity(entity)
            .insert(WriteBytesToSinkRequested(sink));
    }
}

fn load_mft_file_on_bytes_received(
    trigger: On<BytesReceived>,
    goal: Res<LoadCachedMftFilesGoal>,
    mut commands: Commands,
    sinks: Query<&MftFileBytesSink>,
    is_mft_entities: Query<(), With<IsMftFile>>,
    bytes: Query<&BytesHolder>,
    existing: Query<(), With<MftFile>>,
) {
    if !goal.enabled {
        return;
    }

    let entity = trigger.event().entity;

    if let Ok(sink) = sinks.get(entity) {
        let source = sink.source;
        if existing.get(source).is_ok() {
            return;
        }

        let Ok(bytes_holder) = bytes.get(entity) else {
            warn!(?entity, "BytesReceived sink missing BytesHolder");
            return;
        };

        let bytes_clone = bytes_holder.bytes.clone();
        let bytes_vec = bytes_holder.bytes.to_vec();
        commands
            .entity(source)
            .insert(BytesHolder { bytes: bytes_clone });

        match MftFile::from_bytes(bytes_vec) {
            Ok(mft) => {
                info!(?source, mft = ?mft, "Constructed MFT from cached bytes");
                commands.entity(source).insert(mft);
            }
            Err(error) => {
                warn!(?source, ?error, "Failed to construct MFT from bytes");
            }
        }

        commands
            .entity(entity)
            .remove::<MftFileBytesSink>()
            .remove::<BytesHolder>();

        return;
    }

    if is_mft_entities.get(entity).is_err() {
        return;
    }
    if existing.get(entity).is_ok() {
        return;
    }

    let Ok(bytes_holder) = bytes.get(entity) else {
        warn!(?entity, "BytesReceived for MFT entity without BytesHolder");
        return;
    };

    match MftFile::from_bytes(bytes_holder.bytes.to_vec()) {
        Ok(mft) => {
            info!(?entity, mft = ?mft, "Constructed MFT from cached bytes");
            commands.entity(entity).insert(mft);
        }
        Err(error) => {
            warn!(?entity, ?error, "Failed to construct MFT from bytes");
        }
    }
}

fn is_mft_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("mft"))
        .unwrap_or(false)
}
