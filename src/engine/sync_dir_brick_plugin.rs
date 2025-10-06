use crate::engine::sync_dir_plugin::SyncDirectory;
use bevy::prelude::*;
use crate::engine::mft_file_plugin::LoadCachedMftFilesGoal;

#[derive(Component)]
pub struct BaseMaterial(Handle<StandardMaterial>);

#[derive(Component)]
pub struct HoverMaterial(Handle<StandardMaterial>);

pub struct SyncDirBrickPlugin;

impl Plugin for SyncDirBrickPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(spawn_brick_for_new_sync_dirs);
        app.add_observer(on_sync_dir_click);
        app.add_observer(on_sync_dir_hover);
        app.add_observer(on_sync_dir_hover_out);
    }
}

pub fn spawn_brick_for_new_sync_dirs(
    sync_dir: On<Add, SyncDirectory>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let base_matl = materials.add(Color::srgb(1.0, 1.0, 0.0)); // Yellow
    let hover_matl = materials.add(Color::srgb(0.0, 1.0, 1.0)); // Cyan
    commands.entity(sync_dir.entity).insert((
        Name::new("Sync Directory Brick"),
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(base_matl.clone()),
        BaseMaterial(base_matl),
        HoverMaterial(hover_matl),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Pickable::default(),
        Visibility::default(),
    ));
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