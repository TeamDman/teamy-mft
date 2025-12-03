use crate::engine::assets::objects::MyObject;
use crate::engine::search_window_plugin::SearchWindowToggleEvent;
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy::scene::SceneRoot;

#[derive(Component)]
pub struct MagnifyingGlass;

pub struct MagnifyingGlassPlugin;

impl Plugin for MagnifyingGlassPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_magnifying_glass);
        app.add_observer(on_magnifying_glass_click);
    }
}

fn spawn_magnifying_glass(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        Name::new("Magnifying Glass"),
        MagnifyingGlass,
        SceneRoot(
            asset_server
                .load(GltfAssetLabel::Scene(0).from_asset(MyObject::GoldenPlatedMagnifyingGlass)),
        ),
        Transform::from_xyz(-2.0, 0.5, 0.0).with_scale(Vec3::splat(1.0)),
        GlobalTransform::default(),
        Visibility::default(),
        InheritedVisibility::default(),
        Pickable::default(),
    ));
}

fn on_magnifying_glass_click(
    trigger: On<Pointer<Press>>,
    magnifying_glasses: Query<(), With<MagnifyingGlass>>,
    mut commands: Commands,
) {
    if trigger.button != PointerButton::Primary {
        return;
    }

    if magnifying_glasses.get(trigger.event_target()).is_ok() {
        commands.trigger(SearchWindowToggleEvent);
    }
}
