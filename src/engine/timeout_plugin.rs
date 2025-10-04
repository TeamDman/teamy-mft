use bevy::prelude::*;
use std::time::Duration;

pub struct TimeoutPlugin;

impl Plugin for TimeoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            exit_on_timeout.run_if(|timer: Option<Res<ExitTimer>>| timer.is_some()),
        );
    }
}

#[derive(Resource, Debug, Reflect)]
#[reflect(Resource)]
pub struct ExitTimer(pub Timer);

impl ExitTimer {
    pub fn new(duration: Duration) -> Self {
        Self(Timer::new(duration, TimerMode::Once))
    }
}

impl From<Duration> for ExitTimer {
    fn from(duration: Duration) -> Self {
        Self::new(duration)
    }
}

/// When present, the timeout will only log a warning instead of exiting the app
#[derive(Resource, Debug, Reflect, Default)]
#[reflect(Resource)]
pub struct KeepOpen;

fn exit_on_timeout(
    mut timer: ResMut<ExitTimer>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    just_log: Option<Res<KeepOpen>>,
) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        warn!("Test timed out");
        if just_log.is_none() {
            exit.write(AppExit::error());
        } else {
            warn!("Not exiting because ExitTimerJustLog resource is present");
        }
    }
}
