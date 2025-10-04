use crate::engine::file_contents_plugin::FileContents;
use bevy::prelude::*;
use bytes::Bytes;
use std::fmt;
use std::str;

pub struct FileTextPlugin;

impl Plugin for FileTextPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<FileTextContents>();
        app.register_type::<TryInterpretAsText>();
        app.register_type::<FileTextContentsError>();
        app.add_systems(Update, interpret_file_contents_as_text);
        app.add_observer(cleanup);
    }
}

#[derive(Component, Debug, Default, Reflect)]
#[reflect(Component, Default)]
pub struct TryInterpretAsText;

#[derive(Component, Debug, Reflect)]
#[reflect(Component)]
pub struct FileTextContentsError {
    pub error: String,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct FileTextContents {
    #[reflect(ignore)]
    bytes: Bytes,
}

impl fmt::Debug for FileTextContents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileTextContents")
            .field("len", &self.bytes.len())
            .finish()
    }
}

impl FileTextContents {
    pub fn try_from_bytes(bytes: Bytes) -> Result<Self, str::Utf8Error> {
        str::from_utf8(&bytes)?;
        Ok(Self { bytes })
    }

    pub fn try_from_file_contents(contents: &FileContents) -> Result<Self, str::Utf8Error> {
        Self::try_from_bytes(contents.bytes().clone())
    }

    pub fn as_str(&self) -> &str {
        // Safety: construction validates UTF-8
        unsafe { str::from_utf8_unchecked(&self.bytes) }
    }

    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }
}

impl TryFrom<&FileContents> for FileTextContents {
    type Error = str::Utf8Error;

    fn try_from(value: &FileContents) -> Result<Self, Self::Error> {
        FileTextContents::try_from_file_contents(value)
    }
}

impl FileTextContentsError {
    pub fn new(error: str::Utf8Error) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}

fn cleanup(removed: On<Remove, FileContents>, mut commands: Commands) {
    commands
        .entity(removed.entity)
        .try_remove::<(FileTextContents, FileTextContentsError)>();
}

fn interpret_file_contents_as_text(
    mut commands: Commands,
    query: Query<
        (Entity, &FileContents),
        (
            With<TryInterpretAsText>,
            Or<(Changed<FileContents>, Added<TryInterpretAsText>)>,
        ),
    >,
) {
    for (entity, contents) in &query {
        let mut entity_commands = commands.entity(entity);
        entity_commands.remove::<FileTextContents>();
        entity_commands.remove::<FileTextContentsError>();

        match FileTextContents::try_from_file_contents(contents) {
            Ok(text_contents) => {
                entity_commands.insert(text_contents);
            }
            Err(error) => {
                entity_commands.insert(FileTextContentsError::new(error));
            }
        }
    }
}
