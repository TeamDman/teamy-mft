use bevy::app::MainScheduleOrder;
use bevy::ecs::schedule::ExecutorKind;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::ecs::system::NonSendMarker;
use bevy::prelude::*;
use bevy_winit::WINIT_WINDOWS;
use std::any::type_name;

pub struct WindowIconPlugin;

impl Plugin for WindowIconPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<WindowIcon>();
        app.add_message::<AddWindowIconWithRetry>();
        app.add_systems(Update, emit_window_icon_requests);

        // Set up a custom schedule to run the window icon update as an exclusive system.
        // We want to avoid `RefCell already borrowed` issues.
        let mut custom_update_schedule = Schedule::new(UpdateWindowIconsSchedule);
        custom_update_schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        app.add_schedule(custom_update_schedule);

        let mut main_schedule_order = app.world_mut().resource_mut::<MainScheduleOrder>();
        main_schedule_order.insert_after(Last, UpdateWindowIconsSchedule);

        app.add_systems(UpdateWindowIconsSchedule, set_window_icon);
    }
}

#[derive(ScheduleLabel, Debug, Hash, PartialEq, Eq, Clone)]
struct UpdateWindowIconsSchedule;

#[derive(Debug, Component, Reflect)]
pub struct WindowIcon {
    pub image: Handle<Image>,
}

#[derive(Debug, Clone, Reflect, Message)]
pub struct AddWindowIconWithRetry {
    pub image: Handle<Image>,
    pub window: Entity,
}

impl WindowIcon {
    pub fn new(image: Handle<Image>) -> Self {
        Self { image }
    }
}

// Sends an AddWindowIconWithRetry message for any entity that just received WindowIcon
fn emit_window_icon_requests(
    mut q: Query<(Entity, &WindowIcon), Added<WindowIcon>>,
    mut writer: MessageWriter<AddWindowIconWithRetry>,
) {
    for (entity, icon) in &mut q {
        debug!("Detected new WindowIcon on {:?}", entity);
        writer.write(AddWindowIconWithRetry {
            image: icon.image.clone(),
            window: entity,
        });
    }
}

fn set_window_icon(
    mut events: ParamSet<(
        MessageReader<AddWindowIconWithRetry>,
        MessageWriter<AddWindowIconWithRetry>,
    )>,
    assets: Res<Assets<Image>>,
    _main_thread_marker: NonSendMarker,
) {
    if events.p0().is_empty() {
        return;
    }
    let outgoing = WINIT_WINDOWS.with_borrow(|windows| {
        let mut outgoing = Vec::new();
        for event in events.p0().read() {
            debug!(?event.window, ?windows, "Handling {}", type_name::<AddWindowIconWithRetry>());

            // Identify window
            let target_window = windows.get_window(event.window);
            let Some(window) = target_window else {
                warn!(
                    ?windows,
                    "Window {:?} does not exist, retrying later...",
                    event.window
                );
                outgoing.push(event.clone());
                continue;
            };

            // Fetch the image asset
            let Some(image) = assets.get(&event.image) else {
                error!(
                    "Image handle {:?} not found in assets, the window will not have our custom icon",
                    event.image
                );
                continue;
            };

            // Acquire pixel data from the image
            let Some(image_data) = image.data.clone() else {
                error!(
                    "Image handle {:?} has no data, the window will not have our custom icon",
                    event.image
                );
                continue;
            };

            // Convert between formats
            let icon = match winit::window::Icon::from_rgba(
                image_data,
                image.texture_descriptor.size.width,
                image.texture_descriptor.size.height,
            ) {
                Ok(icon) => icon,
                Err(e) => {
                    error!("Failed to construct window icon: {:?}", e);
                    continue;
                }
            };

            // Set the window icon
            info!(image_size = ?image.size(), "Setting window icon");
            window.set_window_icon(Some(icon));
        }
        outgoing
    });

    for event in outgoing {
        events.p1().write(event);
    }
}
