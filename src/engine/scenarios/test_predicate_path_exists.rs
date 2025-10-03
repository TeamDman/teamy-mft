use crate::engine::file_metadata_plugin::Exists;
use crate::engine::file_metadata_plugin::NotExists;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::predicate::predicate::DespawnPredicateWhenDone;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateEvaluationRequests;
use crate::engine::predicate::predicate::PredicateOutcomeFailure;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use crate::engine::predicate::predicate::PredicateOutcomeUnknown;
use crate::engine::predicate::predicate_path_exists::PathExistsPredicate;
use crate::engine::timeout_plugin::ExitTimer;
use bevy::prelude::*;
use std::time::Duration;

#[derive(Resource)]
pub struct TestProgress {
    pub existing_path_entity: Entity,
    pub nonexistent_path_entity: Entity,
    pub existing_path_success: bool,
    pub existing_path_failure: bool,
    pub existing_path_unknown: bool,
    pub nonexistent_path_success: bool,
    pub nonexistent_path_failure: bool,
    pub nonexistent_path_unknown: bool,
}

impl TestProgress {
    pub fn new(existing_path_entity: Entity, nonexistent_path_entity: Entity) -> Self {
        Self {
            existing_path_entity,
            nonexistent_path_entity,
            existing_path_success: false,
            existing_path_failure: false,
            existing_path_unknown: false,
            nonexistent_path_success: false,
            nonexistent_path_failure: false,
            nonexistent_path_unknown: false,
        }
    }

    pub fn record_success(&mut self, entity: Entity) {
        if entity == self.existing_path_entity {
            self.existing_path_success = true;
        }
        if entity == self.nonexistent_path_entity {
            self.nonexistent_path_success = true;
        }
    }

    pub fn record_failure(&mut self, entity: Entity) {
        if entity == self.existing_path_entity {
            self.existing_path_failure = true;
        }
        if entity == self.nonexistent_path_entity {
            self.nonexistent_path_failure = true;
        }
    }

    pub fn record_unknown(&mut self, entity: Entity) {
        if entity == self.existing_path_entity {
            self.existing_path_unknown = true;
        }
        if entity == self.nonexistent_path_entity {
            self.nonexistent_path_unknown = true;
        }
    }
}

/// System that checks if completion criteria are met and exits successfully
fn check_completion(
    progress: Res<TestProgress>,
    existing_path: Query<(), With<Exists>>,
    nonexistent_path: Query<(), With<NotExists>>,
    mut exit: MessageWriter<AppExit>,
) {
    let existing_path_has_exists = existing_path.get(progress.existing_path_entity).is_ok();
    let nonexistent_path_has_not_exists = nonexistent_path
        .get(progress.nonexistent_path_entity)
        .is_ok();

    if existing_path_has_exists && nonexistent_path_has_not_exists {
        info!("PathExistsPredicateTest: ✅ PASSED - completion criteria met");
        exit.write(AppExit::Success);
    }
}

pub fn run_path_exists_predicate_test(mut app: App, timeout: Option<Duration>) -> eyre::Result<()> {
    app.insert_resource(ExitTimer::from(
        timeout.unwrap_or_else(|| Duration::from_secs(5)),
    ));

    // Add the completion check system
    app.add_systems(Update, check_completion);

    info!("PathExistsPredicateTest: Starting");

    // Create entity with path that exists (Cargo.toml in project root)
    let existing_path = app.world_mut().spawn(PathBufHolder::new("Cargo.toml")).id();

    // Create entity with path that doesn't exist
    let nonexistent_path = app
        .world_mut()
        .spawn(PathBufHolder::new("this_file_does_not_exist.txt"))
        .id();

    // Insert progress tracker
    app.insert_resource(TestProgress::new(existing_path, nonexistent_path));

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
        .observe(on_predicate_success)
        .observe(on_predicate_failure)
        .observe(on_predicate_unknown)
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
        .observe(on_predicate_success)
        .observe(on_predicate_failure)
        .observe(on_predicate_unknown)
        .id();

    info!("PathExistsPredicateTest: Spawned test entities and predicates");

    // Run the app - it will exit via check_completion or timeout
    let exit_status = app.run();

    if exit_status.is_success() {
        info!("PathExistsPredicateTest: ✅ Test completed successfully");
        return Ok(());
    }

    error!("PathExistsPredicateTest: ❌ FAILED - test timed out before completion");

    if let Some(progress) = app.world().get_resource::<TestProgress>() {
        let existing_path_has_exists = app
            .world()
            .get::<Exists>(progress.existing_path_entity)
            .is_some();
        let nonexistent_path_has_not_exists = app
            .world()
            .get::<NotExists>(progress.nonexistent_path_entity)
            .is_some();

        error!("  existing_path has Exists: {}", existing_path_has_exists);
        error!(
            "  nonexistent_path has NotExists: {}",
            nonexistent_path_has_not_exists
        );
        error!(
            "  progress: existing(success={}, failure={}, unknown={}), nonexistent(success={}, failure={}, unknown={})",
            progress.existing_path_success,
            progress.existing_path_failure,
            progress.existing_path_unknown,
            progress.nonexistent_path_success,
            progress.nonexistent_path_failure,
            progress.nonexistent_path_unknown,
        );
    } else {
        error!("  TestProgress resource missing when inspecting failure state");
    }

    Err(eyre::eyre!(
        "Test timed out before completion criteria were met"
    ))
}

fn on_predicate_success(trigger: On<PredicateOutcomeSuccess>, mut progress: ResMut<TestProgress>) {
    let evaluated = trigger.event().evaluated;
    info!(?evaluated, "PathExistsPredicateTest: Predicate SUCCESS");
    progress.record_success(evaluated);
}

fn on_predicate_failure(trigger: On<PredicateOutcomeFailure>, mut progress: ResMut<TestProgress>) {
    let evaluated = trigger.event().evaluated;
    info!(?evaluated, "PathExistsPredicateTest: Predicate FAILURE");
    progress.record_failure(evaluated);
}

fn on_predicate_unknown(trigger: On<PredicateOutcomeUnknown>, mut progress: ResMut<TestProgress>) {
    let evaluated = trigger.event().evaluated;
    info!(?evaluated, "PathExistsPredicateTest: Predicate UNKNOWN");
    progress.record_unknown(evaluated);
}
