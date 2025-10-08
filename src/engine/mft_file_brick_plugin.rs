use crate::engine::pathbuf_holder_plugin::PathBufHolder;
use crate::engine::sync_dir_brick_plugin::MftBrickContainerRef;
use crate::mft::mft_file::MftFile;
use bevy::ecs::relationship::Relationship;
use bevy::prelude::ChildOf;
use bevy::prelude::*;

#[derive(Component)]
pub struct BaseMaterial(Handle<StandardMaterial>);

#[derive(Component)]
pub struct HoverMaterial(Handle<StandardMaterial>);

pub struct MftFileBrickPlugin;

impl Plugin for MftFileBrickPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(spawn_brick_for_new_mft_files);
        app.add_observer(on_mft_brick_click);
        app.add_observer(on_mft_brick_hover);
        app.add_observer(on_mft_brick_hover_out);
    }
}

pub fn spawn_brick_for_new_mft_files(
    mft_file: On<Add, MftFile>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    holders: Query<&PathBufHolder>,
    existing: Query<&ChildOf, With<MftFile>>,
    parents: Query<&ChildOf>,
    containers: Query<&MftBrickContainerRef>,
) {
    let holder = holders.get(mft_file.entity).unwrap();
    let name = format!("MFT File: {}", holder.as_path().display());
    let base_matl = materials.add(Color::srgba(255., 181. / 255., 0., 102. / 255.));
    let hover_matl = materials.add(Color::srgba(255., 200. / 255., 0., 150. / 255.)); // Brighter and more opaque

    let parent = match parents.get(mft_file.entity) {
        Ok(parent) => parent,
        Err(_) => {
            warn!(entity=?mft_file.entity, "MFT file entity missing Parent when spawning brick");
            return;
        }
    };

    let container = match containers.get(parent.get()) {
        Ok(container) => container.0,
        Err(_) => {
            warn!(
                entity=?mft_file.entity,
                parent=?parent.get(),
                "Sync directory missing MftBrickContainerRef; skipping brick spawn"
            );
            return;
        }
    };

    let x = existing
        .iter()
        .filter(|parent| parent.get() == container)
        .count() as f32;

    commands.entity(container).add_child(mft_file.entity);

    commands.entity(mft_file.entity).insert((
        Name::new(name),
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(base_matl.clone()),
        BaseMaterial(base_matl),
        HoverMaterial(hover_matl),
        Transform::from_xyz(x, 1.5, 0.0),
        Pickable::default(),
    ));
}

pub fn on_mft_brick_click(
    trigger: On<Pointer<Press>>,
    mfts: Query<(&MftFile, &Name)>,
    mut commands: Commands,
) {
    if trigger.button != PointerButton::Primary {
        return;
    }

    if let Ok((mft, name)) = mfts.get(trigger.event_target()) {
        let record_count = mft.record_count();
        info!(?name, record_count, "Clicked");
        commands.spawn_batch(mft.iter_records().map(|record| {
            (
            //     Name::new(format!(
            //     "MFT Record {}",
            //     record.get_record_number()
            // )),
        )
        }));
    }
}

pub fn on_mft_brick_hover(
    trigger: On<Pointer<Over>>,
    mfts: Query<(), With<MftFile>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    hovers: Query<&HoverMaterial>,
) {
    if mfts.get(trigger.event_target()).is_ok() {
        if let Ok(mut mat) = materials.get_mut(trigger.event_target()) {
            if let Ok(hover) = hovers.get(trigger.event_target()) {
                mat.0 = hover.0.clone();
            }
        }
    }
}

pub fn on_mft_brick_hover_out(
    trigger: On<Pointer<Out>>,
    mfts: Query<(), With<MftFile>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    bases: Query<&BaseMaterial>,
) {
    if mfts.get(trigger.event_target()).is_ok() {
        if let Ok(mut mat) = materials.get_mut(trigger.event_target()) {
            if let Ok(base) = bases.get(trigger.event_target()) {
                mat.0 = base.0.clone();
            }
        }
    }
}
