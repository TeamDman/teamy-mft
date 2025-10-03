use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::predicate::predicate::Predicate;
use crate::engine::predicate::predicate::PredicateOutcomeFailure;
use crate::engine::predicate::predicate::PredicateOutcomeSuccess;
use crate::engine::predicate::predicate::RequestPredicateEvaluation;
use crate::engine::predicate::predicate_file_extension::FileExtensionPredicate;
use crate::engine::timeout_plugin::ExitTimer;
use bevy::prelude::*;
use std::collections::HashSet;
use std::time::Duration;

pub fn test_predicate_file_extension(mut app: App, timeout: Option<Duration>) -> eyre::Result<()> {
    app.insert_resource(ExitTimer::from(
        timeout.unwrap_or_else(|| Duration::from_secs(2)),
    ));

    // Track evaluation results using a resource
    #[derive(Resource, Default)]
    struct TestResults {
        txt_success: HashSet<Entity>,
        txt_failure: HashSet<Entity>,
        mft_success: HashSet<Entity>,
        mft_failure: HashSet<Entity>,
    }
    app.init_resource::<TestResults>();

    // Set up the scenario
    let world = app.world_mut();

    // Create a case-insensitive predicate for .txt files
    let predicate_txt = world
        .spawn((
            Name::new("Predicate - .txt extension (case-insensitive)"),
            Predicate,
            FileExtensionPredicate {
                extension: "txt".into(),
                case_insensitive: true,
            },
        ))
        .id();

    // Create a case-sensitive predicate for .MFT files
    let predicate_mft = world
        .spawn((
            Name::new("Predicate - .MFT extension (case-sensitive)"),
            Predicate,
            FileExtensionPredicate {
                extension: "MFT".into(),
                case_insensitive: false,
            },
        ))
        .id();

    let txt_lowercase = world
        .spawn((
            Name::new("File 1 - document.txt"),
            PathBufHolder::new("document.txt"),
        ))
        .id();
    let txt_uppercase = world
        .spawn((
            Name::new("File 2 - README.TXT (uppercase)"),
            PathBufHolder::new("README.TXT"),
        ))
        .id();
    let png_file = world
        .spawn((
            Name::new("File 3 - image.png"),
            PathBufHolder::new("image.png"),
        ))
        .id();
    let mft_uppercase = world
        .spawn((
            Name::new("File 4 - data.MFT (uppercase)"),
            PathBufHolder::new("data.MFT"),
        ))
        .id();
    let mft_lowercase = world
        .spawn((
            Name::new("File 5 - data.mft (lowercase)"),
            PathBufHolder::new("data.mft"),
        ))
        .id();

    // Add launch conditions
    app.add_systems(Startup, move |mut commands: Commands| {
        // Test case-insensitive .txt predicate
        commands.trigger(RequestPredicateEvaluation {
            predicate: predicate_txt,
            to_evaluate: [txt_lowercase, txt_uppercase, png_file].into(),
        });

        // Test case-sensitive .MFT predicate
        commands.trigger(RequestPredicateEvaluation {
            predicate: predicate_mft,
            to_evaluate: [mft_uppercase, mft_lowercase].into(),
        });
    });

    // Add observers to track results
    app.add_observer(
        move |trigger: On<PredicateOutcomeSuccess>, mut results: ResMut<TestResults>| {
            let event = trigger.event();
            if event.predicate == predicate_txt {
                results.txt_success.insert(event.evaluated);
            } else if event.predicate == predicate_mft {
                results.mft_success.insert(event.evaluated);
            }
        },
    );

    app.add_observer(
        move |trigger: On<PredicateOutcomeFailure>, mut results: ResMut<TestResults>| {
            let event = trigger.event();
            if event.predicate == predicate_txt {
                results.txt_failure.insert(event.evaluated);
            } else if event.predicate == predicate_mft {
                results.mft_failure.insert(event.evaluated);
            }
        },
    );

    // Add success condition
    app.add_systems(
        Update,
        move |results: Res<TestResults>, mut exit: MessageWriter<AppExit>| {
            // Check txt predicate results (case-insensitive)
            let txt_correct = results.txt_success.contains(&txt_lowercase) // document.txt -> pass
                && results.txt_success.contains(&txt_uppercase) // README.TXT -> pass (case-insensitive)
                && results.txt_failure.contains(&png_file); // image.png -> fail

            // Check mft predicate results (case-sensitive)
            let mft_correct = results.mft_success.contains(&mft_uppercase) // data.MFT -> pass (matches exactly)
                && results.mft_failure.contains(&mft_lowercase); // data.mft -> fail (case-sensitive)

            if txt_correct && mft_correct {
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
    use super::test_predicate_file_extension;
    use crate::engine::construction::AppConstructionExt;
    use crate::init_tracing;
    use bevy::prelude::*;
    use tracing::Level;

    #[test]
    fn test_predicate_file_extension_headless() -> eyre::Result<()> {
        init_tracing(Level::INFO);
        test_predicate_file_extension(App::new_headless()?, None)
    }
}
