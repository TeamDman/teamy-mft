use crate::engine::file_metadata_plugin::Exists;
use crate::engine::file_metadata_plugin::NotExists;
use crate::engine::file_metadata_plugin::RequestFileMetadata;
use crate::engine::file_metadata_plugin::RequestFileMetadataInProgress;
use crate::engine::predicate::predicate::LastUsedAt;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateEvaluationRequests;
use crate::engine::predicate::predicate::PredicateOutcomeFailure;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use crate::engine::predicate::predicate::PredicateOutcomeUnknown;
use bevy::prelude::*;
use std::time::Instant;

pub struct PathExistsPredicatePlugin;

impl Plugin for PathExistsPredicatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, evaluate);
    }
}

#[derive(Component, Debug, Reflect, Default)]
pub struct PathExistsPredicate;

/// System that evaluates path existence once metadata is available.
fn evaluate(
    mut predicates: Query<
        (
            Entity,
            &PathExistsPredicate,
            &mut PredicateEvaluationRequests,
        ),
        With<Predicate>,
    >,
    exists: Query<(), With<Exists>>,
    not_exists: Query<(), With<NotExists>>,
    requested: Query<
        (),
        Or<(
            With<RequestFileMetadata>,
            With<RequestFileMetadataInProgress>,
        )>,
    >,
    mut commands: Commands,
) {
    for (predicate, _rule, mut requests) in predicates.iter_mut() {
        debug!(?predicate, request_count=?requests.to_evaluate.len(), "PathExistsPredicate evaluating");
        // Process entities that have metadata available
        let mut to_remove = Vec::new();
        let mut requested_metadata = false;

        for evaluated in requests.to_evaluate.iter() {
            let evaluated = *evaluated;
            if exists.contains(evaluated) {
                commands.trigger(PredicateOutcomeSuccess {
                    predicate,
                    evaluated,
                });
                to_remove.push(evaluated);
                continue;
            }

            if not_exists.contains(evaluated) {
                commands.trigger(PredicateOutcomeFailure {
                    predicate,
                    evaluated,
                });
                to_remove.push(evaluated);
                continue;
            }

            if requested.contains(evaluated) {
                // Metadata request is queued or in progress; wait for completion
                continue;
            }

            // No metadata available yet; request it now
            debug!(
                ?evaluated,
                "Requesting file metadata for path exists evaluation"
            );
            commands.entity(evaluated).insert(RequestFileMetadata);
            requested_metadata = true;
        }

        // Remove evaluated entities from the queue
        let did_work = !to_remove.is_empty();
        for evaluated in to_remove {
            requests.to_evaluate.remove(&evaluated);
        }

        if did_work || requested_metadata {
            commands
                .entity(predicate)
                .insert(LastUsedAt(Instant::now()));
        }

        if !did_work && !requested_metadata {
            // Nothing was resolved or requested; emit unknown outcomes for remaining entities
            let remaining: Vec<_> = requests.to_evaluate.iter().copied().collect();
            for evaluated in remaining {
                if exists.get(evaluated).is_err()
                    && not_exists.get(evaluated).is_err()
                    && requested.get(evaluated).is_err()
                {
                    commands.trigger(PredicateOutcomeUnknown {
                        predicate,
                        evaluated,
                    });
                    requests.to_evaluate.remove(&evaluated);
                }
            }
        }
    }
}
