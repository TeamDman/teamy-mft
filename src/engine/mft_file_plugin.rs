use crate::engine::pathbuf_holder_plugin::PathBufHolder;
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
                on_sync_dir_added_emit_loads,
                handle_mft_file_messages,
                finish_mft_file_tasks,
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

pub fn on_sync_dir_added_emit_loads(
    mut messages: ResMut<Messages<MftFileMessage>>,
    q_added_sync: Query<&PathBufHolder, Added<SyncDirectory>>,
) -> Result<()> {
    for sync_dir in &q_added_sync {
        let dir = match &**sync_dir {
            Some(p) => p,
            None => {
                warn!("SyncDirectory added but has no path; ignoring");
                continue;
            }
        };
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    // only enqueue .mft files
                    let is_mft = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("mft"))
                        .unwrap_or(false);
                    if is_mft && path.is_file() {
                        info!(?path, "Queueing MFT load from path");
                        messages.write(MftFileMessage::LoadFromPath(path));
                    }
                }
            }
            Err(e) => {
                warn!(?dir, error=?e, "Failed to read sync directory");
            }
        }
    }
    Ok(())
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
