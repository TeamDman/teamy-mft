use crate::engine::timeout_plugin::ExitTimer;
use bevy::prelude::*;
use std::time::Duration;

pub fn timeout_situation(mut app: App, timeout: Option<Duration>) -> eyre::Result<()> {
    app.insert_resource(ExitTimer::from(
        timeout.unwrap_or_else(|| Duration::from_secs(1)),
    ));

    // Run until termination, there is no other exit condition so this should time out
    assert!(!app.run().is_success());

    Ok(())
}

#[cfg(test)]
mod test {
    use super::timeout_situation;
    use crate::cli::json_log_behaviour::JsonLogBehaviour;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn timeout_situation_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO, JsonLogBehaviour::None)?;
        timeout_situation(App::new_headless()?, None)
    }
}
