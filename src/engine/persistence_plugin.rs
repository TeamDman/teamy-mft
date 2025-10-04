use crate::engine::file_contents_plugin::FileContents;
use crate::engine::file_contents_plugin::FileContentsInProgress;
use crate::engine::file_contents_plugin::RequestReadFileBytes;
use crate::engine::file_contents_plugin::RequestWriteFileBytes;
use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use bevy::asset::ron;
use bevy::asset::ron::ser::PrettyConfig;
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
    pub config: PersistencePluginConfig<T>,
}
impl<T> Default for PersistencePlugin<T>
where
    T: Persistable,
{
    fn default() -> Self {
        Self {
            config: PersistencePluginConfig::<T>::default(),
        }
    }
}

impl<T: Persistable> Plugin for PersistencePlugin<T> {
    fn build(&self, app: &mut App) {
        app.insert_resource(PersistencePluginConfig::<T> {
            autosave_timer: self.config.autosave_timer.clone(),
            _marker: std::marker::PhantomData,
        });
        app.register_type::<PersistenceKey<T>>();
        app.register_type::<PersistenceProperty<T>>();
        app.register_type::<PersistenceChangedFlag<T>>();
        app.register_type::<PersistenceLoad<T>>();
        app.add_systems(Update, autosave_initiator::<T>);
        app.add_systems(Update, mark_autosave::<T>);
        app.add_systems(Update, autoload_initiator::<T>);
        app.add_systems(Update, cleanup_persistence_file_requests::<T>);
        app.add_systems(Update, autoload_completer::<T>);
    }
}

#[derive(Resource, Debug, Clone)]
pub struct PersistencePluginConfig<T: Persistable> {
    pub autosave_timer: Timer,
    #[allow(dead_code)]
    _marker: std::marker::PhantomData<T>,
}

impl<T: Persistable> Default for PersistencePluginConfig<T> {
    fn default() -> Self {
        Self {
            autosave_timer: Timer::new(Duration::from_millis(5000), TimerMode::Repeating),
            _marker: std::marker::PhantomData,
        }
    }
}

pub trait Persistable:
    'static
    + Send
    + Sync
    + FromReflect
    + TypePath
    + Typed
    + GetTypeRegistration
    + Debug
    + PartialEq
    + Clone
{
    fn serialize(&self, writer: &mut dyn std::io::Write, registry: &TypeRegistry) -> Result<()> {
        let reflect_serializer = TypedReflectSerializer::new(self, registry);
        let output = ron::ser::to_string_pretty(&reflect_serializer, PrettyConfig::default())?;
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

/// Marker component that triggers loading of persistent data.
/// When detected on an entity with a PersistenceKey<T>, will initiate the loading chain.
#[derive(Debug, Component, Reflect)]
pub struct PersistenceLoad<T: Persistable> {
    #[reflect(ignore)]
    pub _marker: std::marker::PhantomData<T>,
}

impl<T: Persistable> Default for PersistenceLoad<T> {
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

/// Debounce variant of PersistenceLoad to indicate that a load is in progress.
#[derive(Debug, Component, Reflect)]
pub struct PersistenceLoadInProgress<T: Persistable> {
    #[reflect(ignore)]
    pub _marker: std::marker::PhantomData<T>,
}
impl<T: Persistable> Default for PersistenceLoadInProgress<T> {
    fn default() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }
}

/// Event triggered when persistent data has been loaded from disk.
#[derive(EntityEvent, Debug, Clone)]
pub struct PersistenceLoaded<T: Persistable> {
    pub entity: Entity,
    pub property: PersistenceProperty<T>,
}

/// Component that tracks which entity is waiting for the file contents being loaded.
/// This is placed on the loader entity to link it back to the waiting entity.
#[derive(Debug, Component, Reflect)]
pub struct PersistenceLoadWaiter<T: Persistable> {
    pub waiter_entity: Entity,
    #[reflect(ignore)]
    pub _marker: std::marker::PhantomData<T>,
}

impl<T: Persistable> PersistenceLoadWaiter<T> {
    pub fn new(waiter_entity: Entity) -> Self {
        Self {
            waiter_entity,
            _marker: std::marker::PhantomData,
        }
    }
}

#[derive(Debug, Component)]
pub struct PersistenceSaveRequest<T: Persistable> {
    pub owner: Entity,
    #[allow(dead_code)]
    pub _marker: std::marker::PhantomData<T>,
}

impl<T: Persistable> PersistenceSaveRequest<T> {
    pub fn new(owner: Entity) -> Self {
        Self {
            owner,
            _marker: std::marker::PhantomData,
        }
    }
}

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
#[derive(Debug, Component, Reflect, Deref, DerefMut, PartialEq, Clone)]
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
pub fn autosave_initiator<T: Persistable>(
    time: Res<Time>,
    mut config: ResMut<PersistencePluginConfig<T>>,
    to_save: Query<
        (Entity, &PersistenceKey<T>, &PersistenceProperty<T>),
        (With<PersistenceChangedFlag<T>>, Without<PersistenceLoad<T>>),
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
            let output_file_path = persistence_directory.join(&key);
            debug!(
                ?entity,
                ?key,
                ?persistence_directory,
                ?prop,
                ?output_file_path,
                "Autosaving property"
            );

            let mut bytes = Vec::new();
            prop.value.serialize(&mut bytes, &registry.read())?;

            debug!(
                ?entity,
                ?key,
                ?output_file_path,
                "Spawning persistence write request"
            );

            commands.spawn((
                Name::new(format!("Autosave - {}", output_file_path.display())),
                PathBufHolder::new(output_file_path.clone()),
                FileContents::new(Bytes::from(bytes)),
                RequestWriteFileBytes,
                PersistenceSaveRequest::<T>::new(entity),
            ));

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

pub fn autoload_initiator<T: Persistable>(
    to_load: Query<
        (Entity, &PersistenceKey<T>),
        (
            With<PersistenceLoad<T>>,
            Without<PersistenceLoadInProgress<T>>,
        ),
    >,
    persistence_directories: Query<&PathBufHolder, With<PersistenceDirectory>>,
    mut commands: Commands,
) -> Result {
    if persistence_directories.is_empty() && !to_load.is_empty() {
        debug!("No PersistenceDirectory found; cannot autoload property - will try again later");
        return Ok(());
    }
    for (entity, key) in to_load.iter() {
        for persistence_directory in persistence_directories.iter() {
            let input_file_path = persistence_directory.join(&key);
            if !input_file_path.exists() {
                warn!(
                    ?entity,
                    ?key,
                    ?persistence_directory,
                    ?input_file_path,
                    "File does not exist; cannot autoload property"
                );
                commands.entity(entity).remove::<PersistenceLoad<T>>();
                continue;
            }
            info!(
                ?entity,
                ?key,
                ?persistence_directory,
                ?input_file_path,
                "Starting autoload property"
            );

            commands.spawn((
                Name::new(format!("Autoload - {}", input_file_path.display())),
                PathBufHolder::new(input_file_path.clone()),
                RequestReadFileBytes,
                PersistenceLoadWaiter::<T>::new(entity),
            ));

            commands
                .entity(entity)
                .insert(PersistenceLoadInProgress::<T>::default()); // Mark the load as in progress
        }
    }
    Ok(())
}

pub fn autoload_completer<T: Persistable>(
    mut commands: Commands,
    loaders: Query<(Entity, &FileContents, &PersistenceLoadWaiter<T>), Added<FileContents>>,
    waiter: Query<&PersistenceKey<T>, With<PersistenceLoad<T>>>,
    registry: Res<AppTypeRegistry>,
) {
    for (loader_entity, contents, load_waiter) in loaders.iter() {
        let waiter_entity = load_waiter.waiter_entity;
        let Ok(waiter_key) = waiter.get(waiter_entity) else {
            warn!(
                ?waiter_entity,
                ?loader_entity,
                "Waiter entity missing PersistenceLoad component during autoload"
            );
            commands.entity(loader_entity).despawn();
            continue;
        };

        debug!(
            ?waiter_entity,
            ?waiter_key,
            "Deserializing loaded file contents"
        );

        let mut cursor = std::io::Cursor::new(contents.bytes().as_ref());
        match T::deserialize(&mut cursor, &registry.read()) {
            Ok(value) => {
                let property = PersistenceProperty::new(value);

                commands.trigger(PersistenceLoaded {
                    entity: waiter_entity,
                    property: property.clone(),
                });

                let mut waiter_commands = commands.entity(waiter_entity);
                waiter_commands.remove::<PersistenceLoad<T>>();
                waiter_commands.remove::<PersistenceLoadInProgress<T>>();
                waiter_commands.insert(property);

                info!(?waiter_entity, ?waiter_key, "Autoload complete");
            }
            Err(error) => {
                warn!(
                    ?waiter_entity,
                    ?waiter_key,
                    ?error,
                    "Failed to deserialize persisted file contents"
                );
            }
        }

        commands.entity(loader_entity).despawn();
    }
}

pub fn cleanup_persistence_file_requests<T: Persistable>(
    mut commands: Commands,
    requests: Query<
        (Entity, &PersistenceSaveRequest<T>),
        (
            Without<FileContentsInProgress>,
            Without<RequestWriteFileBytes>,
            Without<RequestReadFileBytes>,
        ),
    >,
) {
    for (entity, request) in &requests {
        debug!(?entity, owner = ?request.owner, "Cleaning up persistence file request entity");
        commands.entity(entity).despawn();
    }
}
