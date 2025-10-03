use bevy::prelude::*;
use compact_str::CompactString;
use std::time::Duration;

use crate::engine::construction::Testing;

pub struct CleanupPlugin;

impl Plugin for CleanupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, tick_cleanup_countdown);
    }
}

/// Describes why an entity is being scheduled for cleanup.
/// Attached alongside CleanupCountdown to aid debugging.
#[derive(Component, Debug, Reflect)]
pub struct CleanupReason(pub CompactString);

impl CleanupReason {
    pub fn new(reason: impl Into<CompactString>) -> Self {
        Self(reason.into())
    }
}

#[derive(Component, Debug, Reflect)]
pub struct CleanupCountdown {
    pub timer: Timer,
}
impl CleanupCountdown {
    pub fn new(duration: Duration) -> Self {
        Self {
            timer: Timer::new(duration, TimerMode::Once),
        }
    }
}

pub fn tick_cleanup_countdown(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut CleanupCountdown)>,
    testing: Option<Res<Testing>>,
) {
    if testing.is_some() {
        // In testing mode, we don't want automatic cleanup.
        return;
    }
    for (entity, mut countdown) in query.iter_mut() {
        countdown.timer.tick(time.delta());
        if countdown.timer.just_finished() {
            commands.entity(entity).try_despawn();
        }
    }
}
