use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::reflect::TypeRegistration;

pub struct PredicatePlugin;

impl Plugin for PredicatePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_explicit_evaluation_request);
        app.add_systems(Update, despawn_predicates_when_done);
    }
}

/// Marker component for a predicate entity.
/// A predicate entity should have exactly one other component that defines the predicate logic.
#[derive(Component, Debug, Reflect)]
#[require(PredicateEvaluationRequests::default())]
pub struct Predicate;

/// Marker component that causes the predicate entity to be despawned when evaluation is complete.
#[derive(Component, Debug, Reflect, Default)]
pub struct DespawnPredicateWhenDone;

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
    /// The entity that was evaluated
    #[event_target]
    pub entity: Entity,
    /// The predicate entity that performed the evaluation
    pub predicate: Entity,
}

/// Fired when a predicate evaluation fails for an entity.
#[derive(EntityEvent, Debug, Clone)]
pub struct PredicateOutcomeFailure {
    /// The entity that was evaluated
    #[event_target]
    pub entity: Entity,
    /// The predicate entity that performed the evaluation
    pub predicate: Entity,
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

fn despawn_predicates_when_done(
    predicates: Query<
        (Entity, &PredicateEvaluationRequests),
        (
            Changed<PredicateEvaluationRequests>,
            With<Predicate>,
            With<DespawnPredicateWhenDone>,
        ),
    >,
    mut commands: Commands,
) {
    for (entity, requests) in predicates.iter() {
        if !requests.to_evaluate.is_empty() {
            continue;
        }
        debug!(?entity, "Predicate evaluation complete, despawning");
        commands.entity(entity).despawn();
    }
}
