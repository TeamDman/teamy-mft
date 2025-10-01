#![cfg(debug_assertions)]

use crate::engine::write_file_content_plugin::ByteSource;
use crate::engine::write_file_content_plugin::PathBufHolder;
use crate::engine::write_file_content_plugin::WriteBytesToSink;
use bevy::prelude::*;

pub fn test_write_bytes_to_file(mut engine: App) -> eyre::Result<()> {
    // Create byte sink
    let tempfile = tempfile::Builder::new()
        .prefix("test_write_file_content_plugin")
        .suffix(".bin")
        .tempfile()?;
    let path = tempfile.path().to_path_buf();
    let test_bytes = b"Hello, world!".to_vec();
    let sink_entity = engine
        .world_mut()
        .spawn((PathBufHolder { path }, Name::new("Test Bytes Sink")))
        .id();

    // Create byte source
    engine.world_mut().spawn((
        ByteSource {
            bytes: test_bytes.clone().into(),
        },
        Name::new("Test Bytes Source"),
        WriteBytesToSink(sink_entity),
    ));

    // Add success condition
    engine.add_systems(
        Update,
        |remaining: Query<&WriteBytesToSink>, mut exit: MessageWriter<AppExit>| {
            if remaining.is_empty() {
                exit.write(AppExit::Success);
            }
        },
    );

    // Run until termination
    assert!(engine.run().is_success());

    // Verify file contents
    let written_bytes = std::fs::read(tempfile.path())?;
    assert_eq!(written_bytes, test_bytes);
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::engine::construction::AppConstructionExt;
    use crate::engine::exit_condition::AppExitExt;
    use super::test_write_bytes_to_file;
    use crate::init_tracing;
    use bevy::prelude::*;
    use std::time::Duration;
    use tracing::Level;

    #[test]
    fn test_write_bytes_to_file_headless() -> eyre::Result<()> {
        // Initialize logging
        init_tracing(Level::INFO);

        // Construct the engine
        let mut engine = App::new_headless()?;
        engine.add_timeout_exit_system(Duration::from_secs(2));

        test_write_bytes_to_file(engine)
    }
}
