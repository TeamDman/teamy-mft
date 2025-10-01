use bevy::prelude::*;
use std::time::Duration;

pub struct TimeoutPlugin;

impl Plugin for TimeoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, exit_on_timeout);
    }
}

#[derive(Resource, Debug, Reflect)]
#[reflect(Resource)]
pub struct TimeoutExitConfig {
    pub duration: Duration,
}
impl From<Duration> for TimeoutExitConfig {
    fn from(duration: Duration) -> Self {
        Self { duration }
    }
}

fn exit_on_timeout(
    mut timeout: Local<Option<Timer>>,
    time: Res<Time>,
    config: Res<TimeoutExitConfig>,
    mut exit: MessageWriter<AppExit>,
) {
    let timer = timeout.get_or_insert_with(|| Timer::new(config.duration, TimerMode::Once));
    timer.tick(time.delta());
    if timer.just_finished() {
        warn!("Test timed out waiting for write to complete");
        exit.write(AppExit::error());
    }
}
