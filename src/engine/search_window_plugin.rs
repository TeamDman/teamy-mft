use bevy::prelude::*;
use bevy_inspector_egui::bevy_egui::EguiContexts;
use bevy_inspector_egui::bevy_egui::egui;

#[derive(Event, Debug, Clone, Copy, Default)]
pub struct SearchWindowToggleEvent;

#[derive(Resource, Debug, Default)]
pub struct SearchWindowState {
    is_open: bool,
}

pub struct SearchWindowPlugin;

impl Plugin for SearchWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SearchWindowState>();
        app.add_observer(handle_toggle_event);
        app.add_systems(Startup, initialize_primary_context);
        app.add_systems(Update, render_search_window);
    }
}

fn handle_toggle_event(_event: On<SearchWindowToggleEvent>, mut state: ResMut<SearchWindowState>) {
    state.is_open = !state.is_open;
}

fn initialize_primary_context(mut contexts: EguiContexts) {
    // Ensure the primary window has an egui context since auto-creation is disabled.
    let _ = contexts.ctx_mut();
}

fn render_search_window(mut contexts: EguiContexts, mut state: ResMut<SearchWindowState>) {
    if !state.is_open {
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let mut open = state.is_open;
    egui::Window::new("Search")
        .resizable(true)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label("todo");
        });

    state.is_open = open;
}
