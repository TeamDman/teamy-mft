use bevy::prelude::*;
use bevy_inspector_egui::bevy_egui::EguiGlobalSettings;
use bevy_inspector_egui::bevy_egui::EguiPlugin;

pub struct MyEguiPlugin;

impl Plugin for MyEguiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default());
        app.add_systems(Startup, |mut config: ResMut<EguiGlobalSettings>| {
            config.auto_create_primary_context = false;
            // config.enable_absorb_bevy_input_system = true
        });
    }
}
