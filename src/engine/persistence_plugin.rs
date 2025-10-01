use crate::engine::bytes_plugin::ByteSource;
use crate::engine::bytes_plugin::WriteBytesToSink;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::asset::ron;
use bevy::prelude::*;
use bevy::reflect::GetTypeRegistration;
use bevy::reflect::TypeRegistry;
use bevy::reflect::Typed;
use bevy::reflect::serde::TypedReflectDeserializer;
use bevy::reflect::serde::TypedReflectSerializer;
use bytes::Bytes;
use eyre::OptionExt;
use std::any::TypeId;
use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub struct PersistencePlugin<T: Persistable> {
    pub config: PersistencePluginConfig,
    pub _marker: std::marker::PhantomData<T>,
}
impl<T> Default for PersistencePlugin<T>
where
    T: Persistable,
{
    fn default() -> Self {
        Self {
            config: PersistencePluginConfig::default(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: Persistable> Plugin for PersistencePlugin<T> {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone());
        app.register_type::<PersistenceKey<T>>();
        app.register_type::<PersistenceProperty<T>>();
        app.register_type::<PersistenceChangedFlag<T>>();
        app.add_systems(Update, autosave::<T>);
        app.add_systems(Update, mark_autosave::<T>);
    }
}

#[derive(Resource, Debug, Reflect, Clone)]
#[reflect(Resource)]
pub struct PersistencePluginConfig {
    pub autosave_timer: Timer,
}
impl Default for PersistencePluginConfig {
    fn default() -> Self {
        Self {
            autosave_timer: Timer::new(Duration::from_millis(5000), TimerMode::Repeating),
        }
    }
}

pub trait Persistable:
    'static + Send + Sync + FromReflect + TypePath + Typed + GetTypeRegistration + Debug + PartialEq
{
    fn serialize(&self, writer: &mut dyn std::io::Write, registry: &TypeRegistry) -> Result<()> {
        let reflect_serializer = TypedReflectSerializer::new(self, registry);
        let output = ron::to_string(&reflect_serializer)?;
        writer.write_all(output.as_bytes())?;
        Ok(())
    }
    fn deserialize(reader: &mut dyn std::io::Read, registry: &TypeRegistry) -> Result<Self> {
        let mut s = String::new();
        reader.read_to_string(&mut s)?;
        let registration = registry
            .get(TypeId::of::<Self>())
            .ok_or_eyre("Type not registered")?;
        let mut deserializer = ron::Deserializer::from_str(&s)?;
        let reflect_deserializer = TypedReflectDeserializer::new(registration, registry);
        use serde::de::DeserializeSeed;
        let output: Box<dyn PartialReflect> =
            reflect_deserializer.deserialize(&mut deserializer)?;
        assert!(output.as_partial_reflect().represents::<Self>());
        let value: Self = <Self as FromReflect>::from_reflect(output.as_partial_reflect())
            .ok_or_eyre("Failed to deserialize")?;
        Ok(value)
    }
    fn into_persistence_property(self) -> PersistenceProperty<Self>
    where
        Self: Sized,
    {
        PersistenceProperty::new(self)
    }
}

/// Indicates where the persistent data is stored on disk.
#[derive(Debug, Component, Reflect)]
#[require(PathBufHolder)]
pub struct PersistenceDirectory;

/// Identifies that there is persistable data associated with this entity.
/// When the key is present on an entity, the property will be loaded if missing and will be saved on autosave.
#[derive(Debug, Component, Reflect, Deref, DerefMut)]
pub struct PersistenceKey<T: Persistable> {
    /// The key that is joined to the [`PersistenceDestination::path`] to form the full path.
    #[deref]
    pub key: PathBuf,

    #[reflect(ignore)]
    pub _marker: std::marker::PhantomData<T>,
}
impl<T> PersistenceKey<T>
where
    T: Persistable,
{
    pub fn new(key: impl Into<PathBuf>) -> Self {
        Self {
            key: key.into(),
            _marker: std::marker::PhantomData,
        }
    }
}
impl<T: Persistable> AsRef<Path> for PersistenceKey<T> {
    fn as_ref(&self) -> &Path {
        self.key.as_ref()
    }
}

/// The property that is saved and loaded.
#[derive(Debug, Component, Reflect, Deref, DerefMut, PartialEq)]
pub struct PersistenceProperty<T: Persistable> {
    pub value: T,
}
impl<T> PersistenceProperty<T>
where
    T: Persistable,
{
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

/// Indicates that the property has changed and should be saved on the next autosave tick.
#[derive(Debug, Component, Reflect)]
pub struct PersistenceChangedFlag<T: Persistable> {
    #[reflect(ignore)]
    pub _marker: std::marker::PhantomData<T>,
}
impl<T> Default for PersistenceChangedFlag<T>
where
    T: Persistable,
{
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

/// System that autosaves changed properties at intervals defined in the config.
pub fn autosave<T: Persistable>(
    time: Res<Time>,
    mut config: ResMut<PersistencePluginConfig>,
    to_save: Query<
        (Entity, &PersistenceKey<T>, &PersistenceProperty<T>),
        With<PersistenceChangedFlag<T>>,
    >,
    persistence_directories: Query<&PathBufHolder, With<PersistenceDirectory>>,
    mut commands: Commands,
    registry: Res<AppTypeRegistry>,
) -> Result {
    config.autosave_timer.tick(time.delta());
    if !config.autosave_timer.just_finished() {
        return Ok(());
    }
    for (entity, key, prop) in to_save.iter() {
        if persistence_directories.is_empty() {
            warn!(
                ?entity,
                ?key,
                "No PersistenceDirectory found; cannot autosave property"
            );
            continue;
        }
        for persistence_directory in persistence_directories.iter() {
            let output_file_path = persistence_directory.join(&key)?;
            info!(
                ?entity,
                ?key,
                ?persistence_directory,
                ?prop,
                ?output_file_path,
                "Autosaving property"
            );

            let mut bytes = Vec::new();
            prop.value.serialize(&mut bytes, &registry.read())?;
            let bytes = Bytes::from(bytes);

            let sink = commands
                .spawn((
                    Name::new(format!("Autosave sink - {}", output_file_path.display())),
                    PathBufHolder::new(output_file_path),
                ))
                .id();
            commands.spawn((ByteSource { bytes }, WriteBytesToSink(sink)));

            commands
                .entity(entity)
                .remove::<PersistenceChangedFlag<T>>();
        }
    }
    Ok(())
}

/// System that autosaves changed properties at intervals defined in the config.
pub fn mark_autosave<T: Persistable>(
    changed: Query<Entity, Changed<PersistenceProperty<T>>>,
    mut commands: Commands,
) {
    for entity in changed.iter() {
        trace!(?entity, "Marking property as changed for autosave");
        commands
            .entity(entity)
            .insert(PersistenceChangedFlag::<T>::default());
    }
}
