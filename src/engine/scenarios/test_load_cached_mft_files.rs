use crate::engine::directory_children_plugin::RequestDirectoryChildren;
use crate::engine::mft_file_plugin::LoadCachedMftFilesGoal;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::sync_dir_plugin::SyncDirectory;
use crate::engine::timeout_plugin::ExitTimer;
use crate::mft::mft_file::MftFile;
use bevy::prelude::*;
use std::time::Duration;

pub fn test_load_cached_mft_files(mut app: App, timeout: Option<Duration>) -> eyre::Result<()> {
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

    app.world_mut().spawn((
        Name::new(format!(
            "SyncDirectory Test ({})",
            tempdir.path().to_string_lossy()
        )),
        SyncDirectory,
        PathBufHolder::new(tempdir.path()),
        RequestDirectoryChildren,
    ));

    let expected_path = mft_path.clone();
    app.add_systems(
        Update,
        move |query: Query<(&PathBufHolder, &MftFile)>, mut exit: MessageWriter<AppExit>| {
            if query
                .iter()
                .any(|(holder, _)| holder.as_path() == expected_path.as_path())
            {
                exit.write(AppExit::Success);
            }
        },
    );

    assert!(app.run().is_success());
    Ok(())
}

#[cfg(test)]
mod test {
    use super::test_load_cached_mft_files;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn test_load_cached_mft_files_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO);
        test_load_cached_mft_files(App::new_headless()?, None)
    }
}
