use bevy::app::App;
use bevy::app::AppExit;
use bevy::app::Update;
use bevy::ecs::message::MessageWriter;
use bevy::ecs::system::Local;
use bevy::ecs::system::Res;
use bevy::time::Time;
use bevy::time::Timer;
use bevy::time::TimerMode;
use std::time::Duration;
use tracing::warn;

pub trait AppExitExt {
    fn add_timeout_exit_system(&mut self, duration: Duration) -> &mut Self;
}
impl AppExitExt for App {
    fn add_timeout_exit_system(&mut self, duration: Duration) -> &mut Self {
        self.add_systems(
            Update,
            move |mut timeout: Local<Option<Timer>>,
                  time: Res<Time>,
                  mut exit: MessageWriter<AppExit>| {
                let timer = timeout.get_or_insert_with(|| Timer::new(duration, TimerMode::Once));
                timer.tick(time.delta());
                if timer.just_finished() {
                    warn!("Test timed out waiting for write to complete");
                    exit.write(AppExit::error());
                }
            },
        );
        self
    }
}
