use bevy::platform::collections::HashMap;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::reflect::TypeRegistration;
use std::time::Instant;

pub struct PredicatePlugin;

impl Plugin for PredicatePlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_explicit_evaluation_request);
    }
}

/// Marker component for a predicate entity.
/// A predicate entity should have exactly one other component that defines the predicate logic.
#[derive(Component, Debug, Reflect)]
#[require(
    PredicateEvaluationRequests::default(),
    PredicateOutcomeSuccess::default(),
    PredicateOutcomeFailure::default()
)]
pub struct Predicate;

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

/// On a predicate entity: the entities for which this predicate has been evaluated.
#[derive(Component, Debug, Reflect, Default, Deref, DerefMut)]
pub struct PredicateOutcomeSuccess(HashMap<Entity, Instant>);

/// On a predicate entity: the entities for which this predicate has been evaluated.
#[derive(Component, Debug, Reflect, Default, Deref, DerefMut)]
pub struct PredicateOutcomeFailure(HashMap<Entity, Instant>);

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
