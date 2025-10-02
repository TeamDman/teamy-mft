use crate::engine::bytes_plugin::BytesHolder;
use crate::engine::bytes_plugin::BytesReceived;
use crate::engine::bytes_plugin::BytesReceiver;
use crate::engine::bytes_plugin::CleanupOnBytesReceive;
use crate::engine::bytes_plugin::CleanupOnBytesSent;
use crate::engine::bytes_plugin::WriteBytesToSinkRequested;
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
        app.register_type::<PersistenceLoad<T>>();
        app.add_systems(Update, autosave_initiator::<T>);
        app.add_systems(Update, mark_autosave::<T>);
        app.add_systems(Update, autoload_initiator::<T>);
        app.add_observer(autoload_completer::<T>);
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
    mut config: ResMut<PersistencePluginConfig>,
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
            let output_file_path = persistence_directory.join(&key)?;
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
            let bytes = Bytes::from(bytes);

            let sink = commands
                .spawn((
                    Name::new(format!("Autosave sink - {}", output_file_path.display())),
                    PathBufHolder::new(output_file_path.clone()),
                    CleanupOnBytesReceive, // Mark for cleanup when bytes are received
                ))
                .id();
            commands.spawn((
                Name::new(format!("Autosave source - {}", output_file_path.display())),
                BytesHolder { bytes },
                WriteBytesToSinkRequested(sink),
                CleanupOnBytesSent, // Mark for cleanup when bytes are sent
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
            let input_file_path = persistence_directory.join(&key)?;
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

            // Create a BytesReceiver entity that will hold the file contents
            let bytes_receiver = commands
                .spawn((
                    Name::new(format!(
                        "Autoload bytes receiver - {}",
                        input_file_path.display()
                    )),
                    BytesReceiver,
                    CleanupOnBytesReceive, // Mark for cleanup when bytes are received
                ))
                .id();

            // Create a PathBufHolder entity as the source
            commands.spawn((
                Name::new(format!("Autoload source - {}", input_file_path.display())),
                PathBufHolder::new(input_file_path),
                WriteBytesToSinkRequested(bytes_receiver),
                CleanupOnBytesSent, // Mark for cleanup when bytes are sent
            ));

            commands
                .entity(entity)
                .insert(PersistenceLoadInProgress::<T>::default()); // Mark the load as in progress
        }
    }
    Ok(())
}

pub fn autoload_completer<T: Persistable>(
    trigger: On<BytesReceived>,
    sinks: Query<&BytesHolder>,
    waiter: Query<(Entity, &PersistenceKey<T>), With<PersistenceLoad<T>>>,
    mut commands: Commands,
    registry: Res<AppTypeRegistry>,
) {
    let sink = trigger.event().entity;
    let Ok(sink_bytes) = sinks.get(sink) else {
        return;
    };

    debug!(
        ?sink,
        len = sink_bytes.bytes.len(),
        "BytesReceived for autoload"
    );

    // Find which entity is waiting for this data
    // In a real scenario, we'd track the association, but for now we just load for any waiting entity
    for (waiter_entity, waiter_key) in waiter.iter() {
        debug!(?waiter_entity, ?waiter_key, "Deserializing loaded bytes");

        let mut cursor = std::io::Cursor::new(sink_bytes.bytes.as_ref());
        match T::deserialize(&mut cursor, &registry.read()) {
            Ok(value) => {
                let property = PersistenceProperty::new(value);

                // Trigger the loaded event
                commands.trigger(PersistenceLoaded {
                    entity: waiter_entity,
                    property: property.clone(),
                });

                // Remove the load marker
                commands
                    .entity(waiter_entity)
                    .remove::<PersistenceLoad<T>>();
                commands
                    .entity(waiter_entity)
                    .remove::<PersistenceLoadInProgress<T>>();

                info!(?waiter_entity, ?waiter_key, "Autoload complete");

                break; // Only load for the first waiting entity
            }
            Err(error) => {
                warn!(
                    ?waiter_entity,
                    ?waiter_key,
                    ?error,
                    "Failed to deserialize for key"
                );
            }
        }
    }
}
