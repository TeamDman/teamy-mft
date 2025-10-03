use crate::engine::timeout_plugin::TimeoutExitConfig;
use bevy::prelude::*;
use std::time::Duration;

pub fn test_timeout(mut app: App, timeout: Option<Duration>) -> eyre::Result<()> {
    app.insert_resource(TimeoutExitConfig::from(
        timeout.unwrap_or_else(|| Duration::from_secs(1)),
    ));

    // Run until termination, there is no other exit condition so this should time out
    assert!(!app.run().is_success());

    Ok(())
}

#[cfg(test)]
mod test {
    use super::test_timeout;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn test_write_bytes_to_file_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO);
        test_timeout(App::new_headless()?, None)
    }
}
