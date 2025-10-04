use crate::engine::file_contents_plugin::FileContentsInProgress;
use crate::engine::file_contents_plugin::RequestReadFileBytes;
use bevy::prelude::*;
use bevy::time::Timer;
use bevy::time::TimerMode;

pub struct FileContentsRefreshPlugin;

impl Plugin for FileContentsRefreshPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FileContentsRefreshInterval>();
        app.add_systems(Update, refresh_file_contents);
    }
}

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct FileContentsRefreshInterval {
    #[reflect(ignore)]
    timer: Timer,
}

impl FileContentsRefreshInterval {
    pub fn new(timer: Timer) -> Self {
        Self { timer }
    }

    pub fn repeating(seconds: f32) -> Self {
        Self::new(Timer::from_seconds(seconds, TimerMode::Repeating))
    }

    pub fn timer(&self) -> &Timer {
        &self.timer
    }

    pub fn timer_mut(&mut self) -> &mut Timer {
        &mut self.timer
    }
}

fn refresh_file_contents(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut FileContentsRefreshInterval,
        Option<&RequestReadFileBytes>,
        Option<&FileContentsInProgress>,
    )>,
) {
    for (entity, mut interval, has_request, in_progress) in &mut query {
        let timer = interval.timer_mut();
        timer.tick(time.delta());

        if !timer.just_finished() {
            continue;
        }

        if has_request.is_some() || in_progress.is_some() {
            continue;
        }

        commands.entity(entity).insert(RequestReadFileBytes);
    }
}
