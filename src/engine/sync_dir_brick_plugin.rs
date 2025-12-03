use crate::engine::assets::objects::MyObject;
use crate::engine::camera_controller_plugin::FocusTarget;
use crate::engine::camera_controller_plugin::clear_hover_on_exit;
use crate::engine::camera_controller_plugin::store_hover_on_enter;
use crate::engine::mft_file_plugin::LoadCachedMftFilesGoal;
use crate::engine::sync_dir_plugin::SyncDirectory;
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use itertools::Itertools;

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct BaseMaterial(Handle<StandardMaterial>);

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct HoverMaterial(Handle<StandardMaterial>);

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ComputerTowerMaterial;

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ComputerTowerGeometry;

#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ComputerTowerNode;

/// We intentionally spawn the bricks outside of the sync directory entity itself.
/// This is because hover observers traverse the children hierarchy, so it's hard
/// to tell when the sync dir brick is clicked and hovered vs the children.
/// We keep them as children of a container to clean up the inspector hierarchy.
#[derive(Component)]
pub struct MftBrickContainer;

#[derive(Component, Reflect)]
pub struct MftBrickContainerRef(pub Entity);

pub struct SyncDirBrickPlugin;

impl Plugin for SyncDirBrickPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(spawn_brick_for_new_sync_dirs);
        app.add_observer(on_sync_dir_click);
        app.add_observer(on_sync_dir_hover);
        app.add_observer(on_sync_dir_hover_out);
        app.add_observer(on_scene_instance_ready);
    }
}

pub fn spawn_brick_for_new_sync_dirs(
    sync_dir: On<Add, SyncDirectory>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    names: Query<&Name>,
) {
    let base_matl = materials.add(Color::srgb(1.0, 1.0, 0.0)); // Yellow
    let hover_matl = materials.add(Color::srgb(0.0, 1.0, 1.0)); // Cyan

    let container_name = names
        .get(sync_dir.entity)
        .cloned()
        .map(|n| Name::new(format!("MFT Brick Container for {}", n)))
        .unwrap_or_else(|_| Name::new("MFT Brick Container"));

    let container = commands
        .spawn((
            container_name,
            MftBrickContainer,
            Transform::default(),
            GlobalTransform::default(),
            Visibility::default(),
            InheritedVisibility::default(),
        ))
        .id();

    commands
        .entity(sync_dir.entity)
        .insert(MftBrickContainerRef(container));

    commands
        .entity(sync_dir.entity)
        .insert((
            SceneRoot(
                asset_server.load(GltfAssetLabel::Scene(0).from_asset(MyObject::ComputerTower3)),
            ),
            MeshMaterial3d(base_matl.clone()),
            BaseMaterial(base_matl),
            HoverMaterial(hover_matl),
            Transform::from_xyz(0.0, 0.6, 0.0).with_scale(Vec3::splat(1.0)),
            Pickable::default(),
            Visibility::default(),
            FocusTarget,
        ))
        .observe(store_hover_on_enter)
        .observe(clear_hover_on_exit);
}

pub fn on_sync_dir_click(
    trigger: On<Pointer<Press>>,
    sync_dirs: Query<(), With<SyncDirectory>>,
    mut goal: ResMut<LoadCachedMftFilesGoal>,
) {
    if trigger.button != PointerButton::Primary {
        return;
    }

    if sync_dirs.get(trigger.event_target()).is_ok() && !goal.enabled {
        goal.enabled = true;
        info!("Enabled LoadCachedMftFilesGoal by clicking sync directory brick");
    }
}

pub fn on_sync_dir_hover(
    trigger: On<Pointer<Over>>,
    sync_dirs: Query<(), With<SyncDirectory>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    hovers: Query<&HoverMaterial>,
) {
    if sync_dirs.get(trigger.event_target()).is_ok() {
        if let Ok(mut mat) = materials.get_mut(trigger.event_target()) {
            if let Ok(hover) = hovers.get(trigger.event_target()) {
                mat.0 = hover.0.clone();
            }
        }
    }
}

pub fn on_sync_dir_hover_out(
    trigger: On<Pointer<Out>>,
    sync_dirs: Query<(), With<SyncDirectory>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    bases: Query<&BaseMaterial>,
) {
    if sync_dirs.get(trigger.event_target()).is_ok() {
        if let Ok(mut mat) = materials.get_mut(trigger.event_target()) {
            if let Ok(base) = bases.get(trigger.event_target()) {
                mat.0 = base.0.clone();
            }
        }
    }
}

pub fn on_scene_instance_ready(trigger: On<SceneInstanceReady>, spawner: Res<SceneSpawner>) {
    info!(
        ?trigger,
        "Scene instance ready, associated with {:?}",
        spawner
            .iter_instance_entities(trigger.instance_id)
            .collect_vec()
    );
}
