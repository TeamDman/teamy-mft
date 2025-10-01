use bevy::prelude::*;
use bytes::Bytes;

pub struct BytesPlugin;

impl Plugin for BytesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ByteSource>();
        app.register_type::<WriteBytesToSink>();
        app.register_type::<WriteBytesFromSources>();
    }
}

#[derive(Component, Reflect, Debug)]
pub struct ByteSource {
    #[reflect(ignore)]
    pub bytes: Bytes,
}


/// A source can only write to one sink at a time.
/// This is because ByteSource is cheap to clone, so we can just clone it for each sink if needed.
#[derive(Component, Reflect, Debug)]
#[relationship(relationship_target = WriteBytesFromSources)]
pub struct WriteBytesToSink(pub Entity);

/// A sink can have multiple sources writing to it.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
#[relationship_target(relationship = WriteBytesToSink, linked_spawn)]
pub struct WriteBytesFromSources(Vec<Entity>);



/// Debounce variant of WriteBytesToSink/WriteBytesFromSources to indicate that a write is in progress.
#[derive(Component, Reflect, Debug)]
#[relationship(relationship_target = WriteBytesFromSourcesInProgress)]
pub struct WriteBytesToSinkInProgress(pub Entity);

/// Debounce variant of WriteBytesToSink/WriteBytesFromSources to indicate that a write is in progress.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
#[relationship_target(relationship = WriteBytesToSinkInProgress, linked_spawn)]
pub struct WriteBytesFromSourcesInProgress(Vec<Entity>);