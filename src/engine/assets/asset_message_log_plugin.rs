use bevy::prelude::*;

pub struct AssetMessageLogPlugin;

const ENABLED: bool = false;

impl Plugin for AssetMessageLogPlugin {
    fn build(&self, app: &mut App) {
        if !ENABLED {
            return;
        }
        app.add_systems(Update, log_image_events);
    }
}

fn log_image_events(mut messages: MessageReader<AssetEvent<Image>>) {
    for msg in messages.read() {
        debug!(?msg, "Image asset event");
    }
}
