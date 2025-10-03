use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::reflect::TypeRegistration;
use bevy::time::common_conditions::on_timer;
use std::time::Duration;
use std::time::Instant;

pub struct PredicatePlugin;

impl Plugin for PredicatePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_explicit_evaluation_request);
        app.insert_resource(PredicateCleanupConfig {
            staleness_threshold: Duration::from_secs(5),
        });
        app.add_systems(
            Update,
            cleanup_stale_predicates.run_if(on_timer(Duration::from_secs(1))),
        );
    }
}

/// Configuration for automatic predicate cleanup.
#[derive(Resource, Debug)]
pub struct PredicateCleanupConfig {
    pub staleness_threshold: Duration,
}

/// Marker component for a predicate entity.
/// A predicate entity should have exactly one other component that defines the predicate logic.
///
/// Predicates are useful instead of just using Events because it lets us have more control on the load balancing.
/// If we have an expensive operation, we can manipulate the way the predicate empties its request buffer.
/// Even a small amount of work over a large amount of entities can add up, I imagine.
#[derive(Component, Debug, Reflect)]
#[require(PredicateEvaluationRequests::default())]
pub struct Predicate;

/// Marker component that causes the predicate entity to be despawned when evaluation is complete.
#[derive(Component, Debug, Reflect, Default)]
pub struct DespawnPredicateWhenDone;

/// Timestamp of when this predicate last performed work (evaluated entities, emitted outcomes).
/// Updated by predicate evaluation systems when they process entities.
#[derive(Component, Debug, Reflect)]
pub struct LastUsedAt(pub Instant);

/// On a predicate entity: the entities that have requested evaluation of this predicate.
#[derive(Component, Debug, Reflect, Default)]
pub struct PredicateEvaluationRequests {
    pub to_evaluate: HashSet<Entity>,
}

/// Enqueue a request to evaluate a predicate for the given entities.
#[derive(EntityEvent, Debug, Clone)]
pub struct RequestPredicateEvaluation {
    #[event_target]
    pub predicate: Entity,
    pub to_evaluate: HashSet<Entity>,
}

/// Enqueue a request to evaluate a predicate for entities related to the given entities via a specific relationship.
#[derive(EntityEvent, Debug, Clone)]
pub struct RequestPredicateEvaluationForRelatedEntities {
    #[event_target]
    pub predicate: Entity,
    pub discover_from: Entity,
    pub relationship: TypeRegistration,
}

/// Fired when a predicate evaluation succeeds for an entity.
#[derive(EntityEvent, Debug, Clone)]
pub struct PredicateOutcomeSuccess {
    /// The predicate entity that performed the evaluation
    #[event_target]
    pub predicate: Entity,
    /// The entity that was evaluated
    pub evaluated: Entity,
}

/// Fired when a predicate evaluation fails for an entity.
#[derive(EntityEvent, Debug, Clone)]
pub struct PredicateOutcomeFailure {
    /// The predicate entity that performed the evaluation
    #[event_target]
    pub predicate: Entity,
    /// The entity that was evaluated
    pub evaluated: Entity,
}

/// Fired when a predicate evaluation result is unknown (e.g., entity not found or missing required components).
#[derive(EntityEvent, Debug, Clone)]
pub struct PredicateOutcomeUnknown {
    /// The predicate entity that performed the evaluation
    #[event_target]
    pub predicate: Entity,
    /// The entity that was evaluated
    pub evaluated: Entity,
}

fn on_explicit_evaluation_request(
    request: On<RequestPredicateEvaluation>,
    mut predicates: Query<&mut PredicateEvaluationRequests>,
) {
    match predicates.get_mut(request.predicate) {
        Ok(mut requests) => {
            debug!(
                ?request.predicate,
                ?request.to_evaluate,
                "RequestPredicateEvaluation received, added to queue"
            );
            requests.to_evaluate.extend(request.to_evaluate.clone());
        }
        Err(error) => {
            warn!(
                ?request.predicate,
                ?request.to_evaluate,
                ?error,
                "RequestPredicateEvaluation received but could not find predicate"
            );
        }
    }
}

fn cleanup_stale_predicates(
    predicates: Query<
        (Entity, Option<&LastUsedAt>),
        (With<Predicate>, With<DespawnPredicateWhenDone>),
    >,
    config: Res<PredicateCleanupConfig>,
    mut commands: Commands,
) {
    let now = Instant::now();

    for (predicate, last_used) in predicates.iter() {
        let Some(last_used) = last_used else {
            // No LastUsedAt means predicate never did work - skip for now
            continue;
        };

        let age = now.duration_since(last_used.0);
        if age > config.staleness_threshold {
            debug!(
                ?predicate,
                ?age,
                threshold = ?config.staleness_threshold,
                "Predicate is stale, scheduling cleanup"
            );
            commands.entity(predicate).insert((
                crate::engine::cleanup_plugin::CleanupCountdown::new(Duration::ZERO),
                crate::engine::cleanup_plugin::CleanupReason::new(format!(
                    "Predicate stale for {:?}",
                    age
                )),
            ));
        }
    }
}
