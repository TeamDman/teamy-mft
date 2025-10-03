use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateOutcomeFailure;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use crate::engine::predicate::predicate::RequestPredicateEvaluation;
use crate::engine::predicate::predicate_string_ends_with::StringEndsWithPredicate;
use crate::engine::timeout_plugin::TimeoutExitConfig;
use bevy::prelude::*;
use std::time::Duration;

pub fn test_predicate_string_ends_with(
    mut app: App,
    timeout: Option<Duration>,
) -> eyre::Result<()> {
    app.insert_resource(TimeoutExitConfig::from(
        timeout.unwrap_or_else(|| Duration::from_secs(2)),
    ));

    // Set up the scenario
    let world = app.world_mut();
    let predicate = world
        .spawn((
            Name::new("Predicate - Ends with .txt"),
            Predicate,
            StringEndsWithPredicate {
                suffix: ".txt".into(),
            },
        ))
        .id();

    let entity1 = world
        .spawn((
            Name::new("File 1 - document.txt"),
            PathBufHolder::new("document.txt"),
        ))
        .id();
    let entity2 = world
        .spawn((
            Name::new("File 2 - image.png"),
            PathBufHolder::new("image.png"),
        ))
        .id();

    // Add launch conditions
    app.add_systems(Startup, move |mut commands: Commands| {
        commands.trigger(RequestPredicateEvaluation {
            predicate,
            to_evaluate: [entity1, entity2].into(),
        });
    });

    // Add success condition
    app.add_systems(
        Update,
        move |predicate: Single<(&PredicateOutcomeSuccess, &PredicateOutcomeFailure)>,
              mut exit: MessageWriter<AppExit>| {
            let (pass, fail) = *predicate;
            if pass.contains_key(&entity1) && fail.contains_key(&entity2) {
                exit.write(AppExit::Success);
            }
        },
    );

    // Run until termination
    assert!(app.run().is_success());
    Ok(())
}

#[cfg(test)]
mod test {
    use super::test_predicate_string_ends_with;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn test_predicate_string_ends_with_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO);
        test_predicate_string_ends_with(App::new_headless()?, None)
    }
}
