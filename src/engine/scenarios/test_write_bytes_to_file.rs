#![cfg(debug_assertions)]

use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::timeout_plugin::TimeoutExitConfig;
use crate::engine::bytes_plugin::BytesHolder;
use crate::engine::bytes_plugin::WriteBytesToSinkRequested;
use bevy::prelude::*;
use std::time::Duration;

pub fn test_write_bytes_to_file(mut app: App) -> eyre::Result<()> {
    app.insert_resource(TimeoutExitConfig::from(Duration::from_secs(2)));

    // Create byte sink
    let tempfile = tempfile::Builder::new()
        .prefix("test_write_file_content_plugin")
        .suffix(".bin")
        .tempfile()?;
    let path = tempfile.path().to_path_buf();
    let test_bytes = b"Hello, world!".to_vec();
    let sink_entity = app
        .world_mut()
        .spawn((PathBufHolder::new(path), Name::new("Test Bytes Sink")))
        .id();

    // Create byte source
    app.world_mut().spawn((
        BytesHolder {
            bytes: test_bytes.clone().into(),
        },
        Name::new("Test Bytes Source"),
        WriteBytesToSinkRequested(sink_entity),
    ));

    // Add success condition
    app.add_systems(
        Update,
        |remaining: Query<&WriteBytesToSinkRequested>, mut exit: MessageWriter<AppExit>| {
            if remaining.is_empty() {
                exit.write(AppExit::Success);
            }
        },
    );

    // Run until termination
    assert!(app.run().is_success());

    // Verify file contents
    let written_bytes = std::fs::read(tempfile.path())?;
    assert_eq!(written_bytes, test_bytes);
    Ok(())
}

#[cfg(test)]
mod test {
    use super::test_write_bytes_to_file;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn test_write_bytes_to_file_headless() -> eyre::Result<()> {
        // Initialize logging
        init_tracing(Level::INFO);

        // Construct the engine
        let engine = App::new_headless()?;

        // Run the test
        test_write_bytes_to_file(engine)
    }
}
