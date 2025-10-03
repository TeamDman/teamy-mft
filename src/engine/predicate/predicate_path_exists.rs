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
        app.add_systems(Update, (request_metadata, evaluate));
    }
}

#[derive(Component, Debug, Reflect, Default)]
pub struct PathExistsPredicate;

/// System that requests metadata for entities queued for evaluation.
fn request_metadata(
    predicates: Query<
        (Entity, &PathExistsPredicate, &PredicateEvaluationRequests),
        With<Predicate>,
    >,
    to_evaluate: Query<
        (),
        (
            Without<Exists>,
            Without<NotExists>,
            Without<RequestFileMetadata>,
            Without<RequestFileMetadataInProgress>,
        ),
    >,
    mut commands: Commands,
) {
    for (predicate, _rule, requests) in predicates.iter() {
        debug!(?predicate, request_count=?requests.to_evaluate.len(), "PathExistsPredicate processing requests");
        let mut did_work = false;
        for evaluated in requests.to_evaluate.iter() {
            // Only request metadata if we don't already have it
            if to_evaluate.get(*evaluated).is_ok() {
                debug!(?evaluated, "Requesting file metadata for path exists evaluation");
                commands.entity(*evaluated).insert(RequestFileMetadata);
                did_work = true;
            }
        }
        
        if did_work {
            commands.entity(predicate).insert(LastUsedAt(Instant::now()));
        }
    }
}

/// System that evaluates path existence once metadata is available.
fn evaluate(
    mut predicates: Query<
        (Entity, &PathExistsPredicate, &mut PredicateEvaluationRequests),
        With<Predicate>,
    >,
    exists: Query<(), With<Exists>>,
    not_exists: Query<(), With<NotExists>>,
    requested: Query<(), Or<(With<RequestFileMetadata>, With<RequestFileMetadataInProgress>)>>,
    mut commands: Commands,
) {
    for (predicate, _rule, mut requests) in predicates.iter_mut() {
        debug!(?predicate, request_count=?requests.to_evaluate.len(), "PathExistsPredicate evaluating");
        // Process entities that have metadata available
        let mut to_remove = Vec::new();
        
        for evaluated in requests.to_evaluate.iter() {
            // Skip if metadata fetch is still in progress or queued
            if requested.get(*evaluated).is_ok() {
                continue;
            }
            
            // Check if we have existence information
            if exists.get(*evaluated).is_ok() {
                commands.trigger(PredicateOutcomeSuccess { predicate, evaluated: *evaluated });
                to_remove.push(*evaluated);
            } else if not_exists.get(*evaluated).is_ok() {
                commands.trigger(PredicateOutcomeFailure { predicate, evaluated: *evaluated });
                to_remove.push(*evaluated);
            } else {
                // No metadata components present and not in progress - unknown state
                commands.trigger(PredicateOutcomeUnknown { predicate, evaluated: *evaluated });
                to_remove.push(*evaluated);
            }
        }
        
        // Remove evaluated entities from the queue
        let did_work = !to_remove.is_empty();
        for evaluated in to_remove {
            requests.to_evaluate.remove(&evaluated);
        }
        
        if did_work {
            commands.entity(predicate).insert(LastUsedAt(Instant::now()));
        }
    }
}

