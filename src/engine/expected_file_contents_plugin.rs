use crate::engine::file_contents_plugin::FileContents;
use bevy::prelude::*;
use bytes::Bytes;

pub struct ExpectedFileContentsPlugin;

impl Plugin for ExpectedFileContentsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ExpectedFileContents>();
        app.register_type::<HasCorrectFileContents>();
        app.add_systems(
            Update,
            (
                evaluate_expected_on_file_contents_changes,
                evaluate_expected_on_expected_changes,
            ),
        );
        app.add_observer(on_file_contents_removed);
    }
}

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct ExpectedFileContents {
    #[reflect(ignore)]
    bytes: Bytes,
}

impl ExpectedFileContents {
    pub fn new(bytes: Bytes) -> Self {
        Self { bytes }
    }

    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self::new(Bytes::from(bytes))
    }

    pub fn as_bytes(&self) -> &Bytes {
        &self.bytes
    }
}

#[derive(Component, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct HasCorrectFileContents;

fn evaluate_expected_on_file_contents_changes(
    entities: Query<
        Entity,
        (
            With<ExpectedFileContents>,
            Or<(Changed<FileContents>, Added<FileContents>)>,
        ),
    >,
    expected_query: Query<&ExpectedFileContents>,
    file_contents: Query<&FileContents>,
    mut commands: Commands,
) {
    for entity in &entities {
        evaluate_expected(entity, &expected_query, &file_contents, &mut commands);
    }
}

fn evaluate_expected_on_expected_changes(
    entities: Query<
        Entity,
        Or<(Changed<ExpectedFileContents>, Added<ExpectedFileContents>)>,
    >,
    expected_query: Query<&ExpectedFileContents>,
    file_contents: Query<&FileContents>,
    mut commands: Commands,
) {
    for entity in &entities {
        evaluate_expected(entity, &expected_query, &file_contents, &mut commands);
    }
}

fn on_file_contents_removed(trigger: On<Remove, FileContents>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .try_remove::<HasCorrectFileContents>();
}

fn evaluate_expected(
    entity: Entity,
    expected_query: &Query<&ExpectedFileContents>,
    file_contents: &Query<&FileContents>,
    commands: &mut Commands,
) {
    let Ok(expected) = expected_query.get(entity) else {
        return;
    };

    let Ok(contents) = file_contents.get(entity) else {
        return;
    };

    let mut entity_commands = commands.entity(entity);
    if contents.bytes() == expected.as_bytes() {
        entity_commands.insert(HasCorrectFileContents);
    } else {
        entity_commands.try_remove::<HasCorrectFileContents>();
    }
}
