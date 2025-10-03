use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateEvaluationRequests;
use crate::engine::predicate::predicate::PredicateOutcomeFailure;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use bevy::prelude::*;
use compact_str::CompactString;
use std::time::Instant;

pub struct StringEndsWithPredicatePlugin;

impl Plugin for StringEndsWithPredicatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, evaluate);
    }
}

#[derive(Component, Debug, Reflect)]
pub struct StringEndsWithPredicate {
    pub suffix: CompactString,
}

fn evaluate(
    mut predicates: Query<
        (
            &StringEndsWithPredicate,
            &mut PredicateEvaluationRequests,
            &mut PredicateOutcomeSuccess,
            &mut PredicateOutcomeFailure,
        ),
        With<Predicate>,
    >,
    to_evaluate: Query<&PathBufHolder>,
) {
    for (predicate, mut requests, mut success, mut failure) in predicates.iter_mut() {
        if requests.to_evaluate.is_empty() {
            continue;
        }
        debug!(
            "Evaluating StringEndsWithPredicate for {} entities",
            requests.to_evaluate.len()
        );
        for entity in requests.to_evaluate.drain() {
            if let Ok(path_holder) = to_evaluate.get(entity) {
                // ugly, inefficient, should use a dedicated FileExtensionPredicate
                let path_str = path_holder.to_string_lossy();
                if path_str.ends_with((&predicate.suffix).as_str()) {
                    success.insert(entity, Instant::now());
                } else {
                    failure.insert(entity, Instant::now());
                }
            } else {
                failure.insert(entity, Instant::now());
            }
        }
    }
}
