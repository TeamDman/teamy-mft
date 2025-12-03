use crate::engine::directory_children_plugin::RequestDirectoryChildren;
use crate::engine::mft_file_plugin::LoadCachedMftFilesGoal;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::sync_dir_plugin::SyncDirectory;
use crate::engine::timeout_plugin::ExitTimer;
use crate::engine::timeout_plugin::KeepOpen;
use crate::mft::mft_file::MftFile;
use bevy::prelude::*;
use std::path::PathBuf;
use std::time::Duration;
use tracing::info;

#[derive(Resource)]
struct ExpectedPath(PathBuf);

pub fn load_cached_mft_files_situation(
    mut app: App,
    timeout: Option<Duration>,
) -> eyre::Result<()> {
    app.insert_resource(ExitTimer::from(
        timeout.unwrap_or_else(|| Duration::from_secs(2)),
    ));
    app.insert_resource(LoadCachedMftFilesGoal { enabled: true });

    let tempdir = tempfile::tempdir()?;
    let mft_path = tempdir.path().join("cached.mft");

    let mut bytes = vec![0u8; 1024];
    bytes[0x1C] = 0x00;
    bytes[0x1D] = 0x04;
    bytes[0x1E] = 0x00;
    bytes[0x1F] = 0x00;
    std::fs::write(&mft_path, &bytes)?;

    app.insert_resource(ExpectedPath(mft_path.clone()));

    app.world_mut().spawn((
        Name::new(format!(
            "SyncDirectory Test ({})",
            tempdir.path().to_string_lossy()
        )),
        SyncDirectory,
        PathBufHolder::new(tempdir.path()),
        RequestDirectoryChildren,
    ));

    app.add_systems(Update, check_success);

    assert!(app.run().is_success());
    Ok(())
}

fn check_success(
    mut has_succeeded: Local<bool>,
    query: Query<(&PathBufHolder, &MftFile)>,
    mut exit: MessageWriter<AppExit>,
    just_log: Option<Res<KeepOpen>>,
    expected: Res<ExpectedPath>,
) {
    if *has_succeeded {
        return;
    }

    if query
        .iter()
        .any(|(holder, _)| holder.as_path() == expected.0.as_path())
    {
        *has_succeeded = true;
        info!("Situation succeeded");
        if just_log.is_none() {
            exit.write(AppExit::Success);
        }
    }
}

#[cfg(test)]
mod test {
    use super::load_cached_mft_files_situation;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn load_cached_mft_files_situation_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO);
        load_cached_mft_files_situation(App::new_headless()?, None)
    }
}
