use crate::engine::expected_file_contents_plugin::ExpectedFileContents;
use crate::engine::expected_file_contents_plugin::ExpectedFileContentsPlugin;
use crate::engine::expected_file_contents_plugin::HasCorrectFileContents;
use crate::engine::file_contents_plugin::FileContents;
use crate::engine::file_contents_plugin::FileContentsInProgress;
use crate::engine::file_contents_plugin::RequestReadFileBytes;
use crate::engine::file_contents_plugin::RequestWriteFileBytes;
use crate::engine::file_contents_refresh_plugin::FileContentsRefreshInterval;
use crate::engine::file_text_plugin::FileTextContents;
use crate::engine::file_text_plugin::TryInterpretAsText;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::timeout_plugin::ExitTimer;
use bevy::prelude::*;
use bytes::Bytes;
use std::fs;
use std::time::Duration;

pub fn test_file_contents_roundtrip(mut app: App, timeout: Option<Duration>) -> eyre::Result<()> {
    app.insert_resource(ExitTimer::from(
        timeout.unwrap_or_else(|| Duration::from_secs(2)),
    ));
    app.add_plugins(ExpectedFileContentsPlugin);

    let to_write = tempfile::NamedTempFile::new()?;
    let to_read = tempfile::NamedTempFile::new()?;

    let content_to_write = "updated text".to_string();
    let content_to_read = "initial bytes".to_string();

    fs::write(to_read.path(), &content_to_read)?;

    let reader_expected = Bytes::copy_from_slice(content_to_read.as_bytes());
    let writer_expected = Bytes::copy_from_slice(content_to_write.as_bytes());

    let reader_entity = app
        .world_mut()
        .spawn((
            Name::new("Reader"),
            PathBufHolder::new(to_read.path().to_path_buf()),
            RequestReadFileBytes,
            TryInterpretAsText,
            ExpectedFileContents::new(reader_expected.clone()),
            TestReader,
        ))
        .id();

    app.world_mut().spawn((
        Name::new("Writer"),
        PathBufHolder::new(to_write.path().to_path_buf()),
        FileContents::from_slice(content_to_write.as_bytes()),
        RequestWriteFileBytes,
        TestWriter,
    ));

    app.world_mut().spawn((
        Name::new("Watcher"),
        PathBufHolder::new(to_write.path().to_path_buf()),
        FileContentsRefreshInterval::repeating(0.05),
        ExpectedFileContents::new(writer_expected.clone()),
        RequestReadFileBytes,
        TestWatcher,
    ));

    app.insert_resource(TestState {
        reader_entity,
        expected_reader_text: content_to_read.clone(),
    });
    app.insert_resource(TestProgress::default());

    app.add_systems(
        Update,
        (monitor_writer_completion, exit_when_expectations_met),
    );

    assert!(app.run().is_success());
    Ok(())
}

#[derive(Component)]
struct TestReader;

#[derive(Component)]
struct TestWriter;

#[derive(Component)]
struct TestWatcher;

#[derive(Resource)]
struct TestState {
    reader_entity: Entity,
    expected_reader_text: String,
}

#[derive(Default, Resource)]
struct TestProgress {
    writer_completed: bool,
}

fn monitor_writer_completion(
    mut commands: Commands,
    mut progress: ResMut<TestProgress>,
    writer_query: Query<
        (
            Option<&RequestWriteFileBytes>,
            Option<&FileContentsInProgress>,
        ),
        With<TestWriter>,
    >,
    watcher_query: Query<Entity, With<TestWatcher>>,
) {
    if progress.writer_completed {
        return;
    }

    let Some((pending_write, in_progress)) = writer_query.iter().next() else {
        return;
    };

    if pending_write.is_some() || in_progress.is_some() {
        return;
    }

    progress.writer_completed = true;
    for watcher in &watcher_query {
        commands.entity(watcher).insert(RequestReadFileBytes);
    }
}

fn exit_when_expectations_met(
    state: Res<TestState>,
    expected_status: Query<Option<&HasCorrectFileContents>, With<ExpectedFileContents>>,
    reader_text: Query<&FileTextContents, With<TestReader>>,
    mut exit: MessageWriter<AppExit>,
) {
    if expected_status.iter().any(|status| status.is_none()) {
        return;
    }

    let Ok(text_contents) = reader_text.get(state.reader_entity) else {
        return;
    };

    if text_contents.as_str() != state.expected_reader_text {
        return;
    }

    exit.write(AppExit::Success);
}

#[cfg(test)]
mod test {
    use super::test_file_contents_roundtrip;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn test_file_contents_roundtrip_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO);
        test_file_contents_roundtrip(App::new_headless()?, None)
    }
}
