use crate::mft::mft_file::MftFile;
use bevy::prelude::*;

pub struct MftFileBrickPlugin;

impl Plugin for MftFileBrickPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(spawn_brick_for_new_mft_files);
    }
}

pub fn spawn_brick_for_new_mft_files(
    mft_file: On<Add, MftFile>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing: Query<Entity, With<MftFile>>,
) {
    let x = existing.iter().count() as f32;
    commands.entity(mft_file.entity).insert((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(Color::srgba(255., 181. / 255., 0., 102. / 255.))),
        Transform::from_xyz(x, 0.5, 0.0),
    ));
}
