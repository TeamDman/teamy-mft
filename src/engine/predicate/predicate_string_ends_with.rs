use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::predicate::predicate::LastUsedAt;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateEvaluationRequests;
use crate::engine::predicate::predicate::PredicateOutcomeFailure;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use crate::engine::predicate::predicate::PredicateOutcomeUnknown;
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
        (Entity, &StringEndsWithPredicate, &mut PredicateEvaluationRequests),
        With<Predicate>,
    >,
    to_evaluate: Query<&PathBufHolder>,
    mut commands: Commands,
) {
    for (predicate, rule, mut requests) in predicates.iter_mut() {
        if requests.to_evaluate.is_empty() {
            continue;
        }
        debug!(
            "Evaluating StringEndsWithPredicate for {} entities",
            requests.to_evaluate.len()
        );
        let mut did_work = false;
        for evaluated in requests.to_evaluate.drain() {
            did_work = true;
            let Ok(path_holder) = to_evaluate.get(evaluated) else {
                commands.trigger(PredicateOutcomeUnknown {
                    predicate,
                    evaluated,
                });
                continue;
            };

            // ugly, inefficient, should use a dedicated FileExtensionPredicate
            let path_str = path_holder.to_string_lossy();
            if path_str.ends_with((&rule.suffix).as_str()) {
                commands.trigger(PredicateOutcomeSuccess {
                    predicate,
                    evaluated,
                });
            } else {
                commands.trigger(PredicateOutcomeFailure {
                    predicate,
                    evaluated,
                });
            }
        }
        
        if did_work {
            commands.entity(predicate).insert(LastUsedAt(Instant::now()));
        }
    }
}
