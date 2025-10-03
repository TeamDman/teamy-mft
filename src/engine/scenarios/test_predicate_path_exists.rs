use bevy::prelude::*;
use std::time::Duration;

use crate::engine::predicate::predicate::{
    DespawnPredicateWhenDone, Predicate, PredicateEvaluationRequests,
    PredicateOutcomeFailure, PredicateOutcomeSuccess,
};
use crate::engine::predicate::predicate_path_exists::PathExistsPredicate;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::timeout_plugin::ExitTimer;

#[derive(Resource, Default)]
pub struct PathExistsPredicateTestResult {
    pub existing_path_succeeded: bool,
    pub existing_path_failed: bool,
    pub nonexistent_path_succeeded: bool,
    pub nonexistent_path_failed: bool,
}

pub fn run_path_exists_predicate_test(
    mut app: App,
    timeout: Option<Duration>,
) -> eyre::Result<()> {
    app.insert_resource(ExitTimer::from(
        timeout.unwrap_or_else(|| Duration::from_secs(5)),
    ));

    info!("PathExistsPredicateTest: Starting");

    app.insert_resource(PathExistsPredicateTestResult::default());

    // Create entity with path that exists (Cargo.toml in project root)
    let existing_path = app
        .world_mut()
        .spawn(PathBufHolder::new("Cargo.toml"))
        .id();

    // Create entity with path that doesn't exist
    let nonexistent_path = app
        .world_mut()
        .spawn(PathBufHolder::new("this_file_does_not_exist.txt"))
        .id();

    // Create predicate for existing path
    let _existing_path_predicate = app
        .world_mut()
        .spawn((
            Name::new("Predicate - Path Exists (Existing)"),
            Predicate,
            PathExistsPredicate,
            DespawnPredicateWhenDone,
            PredicateEvaluationRequests {
                to_evaluate: [existing_path].into_iter().collect(),
            },
        ))
        .observe(on_existing_path_success)
        .observe(on_existing_path_failure)
        .id();

    // Create predicate for nonexistent path
    let _nonexistent_path_predicate = app
        .world_mut()
        .spawn((
            Name::new("Predicate - Path Exists (Nonexistent)"),
            Predicate,
            PathExistsPredicate,
            DespawnPredicateWhenDone,
            PredicateEvaluationRequests {
                to_evaluate: [nonexistent_path].into_iter().collect(),
            },
        ))
        .observe(on_nonexistent_path_success)
        .observe(on_nonexistent_path_failure)
        .id();

    info!("PathExistsPredicateTest: Spawned test entities and predicates");

    // Run the app - it will exit via timeout
    app.run();

    // Check results from world
    let result = app.world().resource::<PathExistsPredicateTestResult>();

    if result.existing_path_succeeded && !result.existing_path_failed
        && result.nonexistent_path_failed && !result.nonexistent_path_succeeded
    {
        info!("PathExistsPredicateTest: ✅ PASSED - both predicates evaluated correctly");
        Ok(())
    } else if result.existing_path_failed || result.nonexistent_path_succeeded {
        error!("PathExistsPredicateTest: ❌ FAILED - predicate evaluation incorrect");
        error!("  existing_path: succeeded={}, failed={}", result.existing_path_succeeded, result.existing_path_failed);
        error!("  nonexistent_path: succeeded={}, failed={}", result.nonexistent_path_succeeded, result.nonexistent_path_failed);
        Err(eyre::eyre!("Predicate evaluation incorrect"))
    } else {
        error!("PathExistsPredicateTest: ❌ FAILED - predicates did not complete evaluation");
        error!("  existing_path: succeeded={}, failed={}", result.existing_path_succeeded, result.existing_path_failed);
        error!("  nonexistent_path: succeeded={}, failed={}", result.nonexistent_path_succeeded, result.nonexistent_path_failed);
        Err(eyre::eyre!("Predicates did not complete evaluation"))
    }
}

fn on_existing_path_success(
    _trigger: On<PredicateOutcomeSuccess>,
    mut test_result: ResMut<PathExistsPredicateTestResult>,
) {
    info!("PathExistsPredicateTest: Existing path SUCCESS (correct)");
    test_result.existing_path_succeeded = true;
}

fn on_existing_path_failure(
    _trigger: On<PredicateOutcomeFailure>,
    mut test_result: ResMut<PathExistsPredicateTestResult>,
) {
    error!("PathExistsPredicateTest: Existing path FAILURE (WRONG - should exist)");
    test_result.existing_path_failed = true;
}

fn on_nonexistent_path_success(
    _trigger: On<PredicateOutcomeSuccess>,
    mut test_result: ResMut<PathExistsPredicateTestResult>,
) {
    error!("PathExistsPredicateTest: Nonexistent path SUCCESS (WRONG - should not exist)");
    test_result.nonexistent_path_succeeded = true;
}

fn on_nonexistent_path_failure(
    _trigger: On<PredicateOutcomeFailure>,
    mut test_result: ResMut<PathExistsPredicateTestResult>,
) {
    info!("PathExistsPredicateTest: Nonexistent path FAILURE (correct)");
    test_result.nonexistent_path_failed = true;
}
