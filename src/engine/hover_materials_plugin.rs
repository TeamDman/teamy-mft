use bevy::pbr::MeshMaterial3d;
use bevy::picking::prelude::*;
use bevy::prelude::*;

pub struct HoverMaterialsPlugin;

impl Plugin for HoverMaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(on_hover_material_over);
        app.add_observer(on_hover_material_out);
    }
}

/// Stores the base and hover materials for a mesh and provides helpers to swap them.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct HoverMaterials {
    pub base: Handle<StandardMaterial>,
    pub hovered: Handle<StandardMaterial>,
}

impl HoverMaterials {
    pub fn new(base: Handle<StandardMaterial>, hovered: Handle<StandardMaterial>) -> Self {
        Self { base, hovered }
    }
}

fn on_hover_material_over(
    trigger: On<Pointer<Over>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    hover_materials: Query<&HoverMaterials>,
) {
    let Ok(hover) = hover_materials.get(trigger.event_target()) else {
        return;
    };
    let Ok(mut mat) = materials.get_mut(trigger.event_target()) else {
        return;
    };
    mat.0 = hover.hovered.clone();
}

fn on_hover_material_out(
    trigger: On<Pointer<Out>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    hover_materials: Query<&HoverMaterials>,
) {
    let Ok(hover) = hover_materials.get(trigger.event_target()) else {
        return;
    };
    let Ok(mut mat) = materials.get_mut(trigger.event_target()) else {
        return;
    };
    mat.0 = hover.base.clone();
}
