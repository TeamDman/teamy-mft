use crate::engine::construction::Headless;
use crate::engine::construction::Testing;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::predicate::predicate::DespawnPredicateWhenDone;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use crate::engine::predicate::predicate::RequestPredicateEvaluation;
use crate::engine::predicate::predicate_file_extension::FileExtensionPredicate;
use crate::engine::sync_dir_plugin::SyncDirectory;
use crate::mft::mft_file::MftFile;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use bevy::tasks::Task;
use bevy::tasks::block_on;
use bevy::tasks::poll_once;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct MftFilePlugin;

impl Plugin for MftFilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MftFileTasks>();
        app.add_message::<MftFileMessage>();
        app.add_systems(
            Update,
            (
                handle_mft_file_messages,
                finish_mft_file_tasks,
                on_sync_dir_child_discovered,
            ),
        );
    }
}

#[derive(Message, Reflect, Clone, Debug)]
#[reflect]
pub enum MftFileMessage {
    LoadFromPath(PathBuf),
}

#[derive(Resource, Default)]
pub struct MftFileTasks {
    pub loading_from_disk: HashMap<PathBuf, Task<Result<MftFile>>>,
}

/// Marker component indicating this PathBufHolder entity needs to have its MFT file loaded.
#[derive(Component, Debug, Reflect, Default)]
pub struct MftFileNeedsLoading;

pub fn on_sync_dir_child_discovered(
    new_children: Query<&Children, (Changed<Children>, With<SyncDirectory>)>,
    headless: Option<Res<Headless>>,
    testing: Option<Res<Testing>>,
    mut commands: Commands,
) {
    if headless.is_some() || testing.is_some() {
        return;
    }
    for children in &new_children {
        // Collect all child entities
        let child_entities: Vec<Entity> = children.iter().collect();

        if child_entities.is_empty() {
            continue;
        }

        debug!(
            "Discovered {} children in sync directory, spawning MFT file extension predicate",
            child_entities.len()
        );

        // Spawn a one-time predicate to filter for .mft files
        let predicate = commands
            .spawn((
                Name::new("MFT File Extension Filter"),
                Predicate,
                DespawnPredicateWhenDone,
                FileExtensionPredicate::new("mft"),
            ))
            .observe(on_mft_predicate_success)
            .id();

        // Request evaluation of all children
        commands.trigger(RequestPredicateEvaluation {
            predicate,
            to_evaluate: child_entities.into_iter().collect(),
        });
    }
}

pub fn handle_mft_file_messages(
    mut reader: MessageReader<MftFileMessage>,
    mut tasks: ResMut<MftFileTasks>,
) -> Result<()> {
    let pool = IoTaskPool::get();
    for msg in reader.read() {
        match msg {
            MftFileMessage::LoadFromPath(path) => {
                if tasks.loading_from_disk.contains_key(path) {
                    warn!(
                        ?path,
                        "LoadFromPath requested but task already running; ignoring"
                    );
                    continue;
                }
                let path_clone = path.clone();
                let task = pool.spawn(async move { Ok(MftFile::from_path(&path_clone)?) });
                debug!(task=?task, path=?path, "Spawned task to load MFT from disk");
                tasks.loading_from_disk.insert(path.clone(), task);
            }
        }
    }
    Ok(())
}

pub fn finish_mft_file_tasks(
    mut commands: Commands,
    mut tasks: ResMut<MftFileTasks>,
) -> Result<()> {
    let mut completed: Vec<PathBuf> = Vec::new();
    for (path, task) in tasks.loading_from_disk.iter_mut() {
        if let Some(result) = block_on(poll_once(task)) {
            match result {
                Ok(mft) => {
                    info!(?path, mft=?format!("{:?}", &mft), "Loaded MFT file from disk");
                    commands.spawn((mft, Name::new(format!("MFT File: {}", path.display()))));
                }
                Err(e) => {
                    warn!(?path, error=?e, "Failed to load MFT file from disk");
                }
            }
            completed.push(path.clone());
        }
    }
    if !completed.is_empty() {
        debug!(
            "Completed {} MFT load tasks, {} remaining",
            completed.len(),
            tasks.loading_from_disk.len() - completed.len()
        );
    }
    for path in completed {
        tasks.loading_from_disk.remove(&path);
    }
    Ok(())
}

/// Observer that responds to successful MFT file extension predicate evaluations.
fn on_mft_predicate_success(
    trigger: On<PredicateOutcomeSuccess>,
    path_holders: Query<&PathBufHolder>,
    mut commands: Commands,
    mut messages: ResMut<Messages<MftFileMessage>>,
) {
    let evaluated = trigger.event().evaluated;

    let Ok(path_holder) = path_holders.get(evaluated) else {
        return;
    };
    // If it is not a file, then that will be handled later.
    let path = path_holder.to_path_buf();
    debug!(
        ?evaluated,
        ?path,
        "MFT file identified, marking for loading"
    );
    commands.entity(evaluated).insert(MftFileNeedsLoading);
    messages.write(MftFileMessage::LoadFromPath(path));
}
