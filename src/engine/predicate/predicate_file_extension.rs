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

pub struct FileExtensionPredicatePlugin;

impl Plugin for FileExtensionPredicatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, evaluate);
    }
}

#[derive(Component, Debug, Reflect)]
pub struct FileExtensionPredicate {
    /// The file extension to match (without the leading dot, e.g., "txt", "mft")
    pub extension: CompactString,
    /// Whether to perform case-insensitive matching
    pub case_insensitive: bool,
}

impl FileExtensionPredicate {
    pub fn new(extension: impl Into<CompactString>) -> Self {
        Self {
            extension: extension.into(),
            case_insensitive: true,
        }
    }
}

fn evaluate(
    mut predicates: Query<
        (
            Entity,
            &FileExtensionPredicate,
            &mut PredicateEvaluationRequests,
        ),
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
            "Evaluating FileExtensionPredicate for {} entities",
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

            // Efficiently get the extension without allocating a full string
            let matches = path_holder
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    if rule.case_insensitive {
                        ext.eq_ignore_ascii_case(rule.extension.as_str())
                    } else {
                        ext == rule.extension.as_str()
                    }
                })
                .unwrap_or(false);

            if matches {
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
            commands
                .entity(predicate)
                .insert(LastUsedAt(Instant::now()));
        }
    }
}
