use crate::engine::cleanup_plugin::CleanupCountdown;
use bevy::prelude::*;
use bytes::Bytes;
use std::time::Duration;

pub struct BytesPlugin;

impl Plugin for BytesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(cleanup_on_bytes_received);
        app.add_observer(cleanup_on_bytes_sent);
    }
}

#[derive(Component, Reflect, Debug)]
pub struct BytesHolder {
    #[reflect(ignore)]
    pub bytes: Bytes,
}

/// A source can only write to one sink at a time.
/// This is because ByteSource is cheap to clone, so we can just clone it for each sink if needed.
#[derive(Component, Reflect, Debug)]
#[relationship(relationship_target = WriteBytesFromSourcesRequested)]
pub struct WriteBytesToSinkRequested(pub Entity);

/// A sink can have multiple sources writing to it.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
#[relationship_target(relationship = WriteBytesToSinkRequested, linked_spawn)]
pub struct WriteBytesFromSourcesRequested(Vec<Entity>);

/// Debounce variant of WriteBytesToSink/WriteBytesFromSources to indicate that a write is in progress.
#[derive(Component, Reflect, Debug)]
#[relationship(relationship_target = WriteBytesFromSourcesInProgress)]
pub struct WriteBytesToSinkInProgress(pub Entity);

/// Debounce variant of WriteBytesToSink/WriteBytesFromSources to indicate that a write is in progress.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
#[relationship_target(relationship = WriteBytesToSinkInProgress, linked_spawn)]
pub struct WriteBytesFromSourcesInProgress(Vec<Entity>);

#[derive(Component, Reflect, Debug)]
#[relationship(relationship_target = WriteBytesFromSourcesFinished)]
pub struct WriteBytesToSinkFinished(pub Entity);

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
#[relationship_target(relationship = WriteBytesToSinkFinished, linked_spawn)]
pub struct WriteBytesFromSourcesFinished(Vec<Entity>);

/// A component that indicates this entity is waiting to receive bytes.
/// When bytes are received, this component is removed and replaced with BytesHolder.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
pub struct BytesReceiver;

/// Event triggered when bytes have been received and written to a BytesReceiver entity.
#[derive(EntityEvent, Debug, Clone, Copy)]
pub struct BytesReceived {
    pub entity: Entity,
}

/// Event triggered when bytes have been sent from a BytesHolder source to a sink.
#[derive(EntityEvent, Debug, Clone, Copy)]
pub struct BytesSent {
    pub entity: Entity,
}

/// A marker component indicating that the entity should be cleaned up (despawned) when bytes are received.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
pub struct CleanupOnBytesReceive;

/// A marker component indicating that the entity should be cleaned up (despawned) when bytes are sent.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Default)]
pub struct CleanupOnBytesSent;

/// Observer that cleans up entities marked with CleanupOnBytesReceive when BytesReceived event is triggered.
fn cleanup_on_bytes_received(
    trigger: On<BytesReceived>,
    query: Query<Entity, With<CleanupOnBytesReceive>>,
    mut commands: Commands,
) {
    if query.get(trigger.event().entity).is_ok() {
        debug!(entity = ?trigger.event().entity, "Cleaning up entity marked with CleanupOnBytesReceive");
        commands
            .entity(trigger.event().entity)
            .insert(CleanupCountdown::new(Duration::from_millis(5000)));
    }
}

/// Observer that cleans up entities marked with CleanupOnBytesSent when BytesSent event is triggered.
fn cleanup_on_bytes_sent(
    trigger: On<BytesSent>,
    query: Query<Entity, With<CleanupOnBytesSent>>,
    mut commands: Commands,
) {
    if query.get(trigger.event().entity).is_ok() {
        debug!(entity = ?trigger.event().entity, "Cleaning up entity marked with CleanupOnBytesSent");
        commands
            .entity(trigger.event().entity)
            .insert(CleanupCountdown::new(Duration::from_millis(5000)));
    }
}
