use crate::engine::assets::textures::MyTexture;
use crate::engine::persistence_plugin::Persistable;
use crate::engine::persistence_plugin::PersistenceKey;
use crate::engine::persistence_plugin::PersistenceLoad;
use crate::engine::persistence_plugin::PersistenceLoaded;
use crate::engine::persistence_plugin::PersistencePlugin;
use crate::engine::persistence_plugin::PersistenceProperty;
use bevy::app::AppExit;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::ui::AlignItems;
use bevy::ui::BackgroundColor;
use bevy::ui::BorderColor;
use bevy::ui::BorderRadius;
use bevy::ui::Interaction;
use bevy::ui::JustifyContent;
use bevy::ui::Node;
use bevy::ui::UiRect;
use bevy::ui::UiTargetCamera;
use bevy::ui::Val;
use bevy::window::WindowIcon;
use bevy::window::WindowRef;
use bevy::window::WindowResolution;

const QUIT_BUTTON_WINDOW_LAYER: usize = 1;

/// Marker component for the quit button window entity
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
pub struct QuitButtonWindow;

#[derive(Component, Reflect, Debug, Default)]
pub struct QuitButtonWindowCamera;

/// Marker component for the quit button
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
pub struct QuitButton;

#[derive(Debug, Reflect, PartialEq, Clone)]
pub struct QuitButtonWindowPersistenceProperty {
    pub position: WindowPosition,
    pub resolution: WindowResolution,
}

impl Persistable for QuitButtonWindowPersistenceProperty {}

impl From<&Window> for QuitButtonWindowPersistenceProperty {
    fn from(window: &Window) -> Self {
        Self {
            position: window.position,
            resolution: window.resolution.clone(),
        }
    }
}

pub struct QuitButtonWindowPlugin;

impl Plugin for QuitButtonWindowPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<QuitButtonWindow>();
        app.register_type::<QuitButtonWindowCamera>();
        app.register_type::<QuitButton>();
        app.add_systems(Startup, spawn_window_if_missing);
        app.add_systems(Update, handle_window_change);
        app.add_systems(Update, update_quit_button_visuals);
        app.add_systems(Update, handle_quit_button);
        app.add_observer(handle_persistence_loaded);
        app.add_plugins(PersistencePlugin::<QuitButtonWindowPersistenceProperty>::default());
    }
}

const DEFAULT_SIZE: UVec2 = UVec2::new(400, 200);

const QUIT_BUTTON_NORMAL: Color = Color::srgb(0.15, 0.15, 0.18);
const QUIT_BUTTON_HOVERED: Color = Color::srgb(0.55, 0.25, 0.32);
const QUIT_BUTTON_PRESSED: Color = Color::srgb(0.35, 0.15, 0.2);

const QUIT_BUTTON_BORDER_NORMAL: Color = Color::srgb(0.75, 0.75, 0.82);
const QUIT_BUTTON_BORDER_HOVERED: Color = Color::srgb(0.95, 0.95, 0.98);
const QUIT_BUTTON_BORDER_PRESSED: Color = Color::srgb(0.95, 0.45, 0.55);

fn spawn_window_if_missing(
    mut commands: Commands,
    existing: Query<Entity, With<QuitButtonWindow>>,
    asset_server: Res<AssetServer>,
) {
    if !existing.is_empty() {
        return;
    }
    let window = commands
        .spawn((
            Name::new("Quit Button Window"),
            Window {
                title: "Quit".into(),
                resolution: WindowResolution::new(DEFAULT_SIZE.x, DEFAULT_SIZE.y),
                ..default()
            },
            QuitButtonWindow,
            WindowIcon {
                handle: asset_server.load(MyTexture::Icon),
            },
            PersistenceKey::<QuitButtonWindowPersistenceProperty>::new(
                "preferences/quit_button_window.ron",
            ),
            PersistenceLoad::<QuitButtonWindowPersistenceProperty>::default(),
        ))
        .id();
    debug!("Spawned Quit Button window");

    let camera = commands
        .spawn((
            Name::new("Quit Button Window Camera"),
            Camera {
                target: RenderTarget::Window(WindowRef::Entity(window)),
                ..default()
            },
            Camera2d,
            QuitButtonWindowCamera,
            RenderLayers::layer(QUIT_BUTTON_WINDOW_LAYER),
        ))
        .id();

    let button_font: Handle<Font> = asset_server.load("fonts/ISOCPEUR.ttf");

    // Spawn the UI
    commands.spawn((
        Name::new("Quit Button UI Root"),
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        UiTargetCamera(camera),
        RenderLayers::layer(QUIT_BUTTON_WINDOW_LAYER),
        children![(
            Name::new("Quit Button"),
            Button,
            QuitButton,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(96.0),
                border: UiRect::all(Val::Px(6.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(QUIT_BUTTON_BORDER_NORMAL),
            BorderRadius::all(Val::Px(24.0)),
            BackgroundColor(QUIT_BUTTON_NORMAL),
            children![(
                Text::new("Quit"),
                TextFont {
                    font: button_font.clone(),
                    font_size: 48.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                TextShadow::default(),
            )]
        )],
    ));
}

fn handle_window_change(
    changed: Query<
        (
            Entity,
            &Window,
            Option<&PersistenceProperty<QuitButtonWindowPersistenceProperty>>,
        ),
        (Changed<Window>, With<QuitButtonWindow>),
    >,
    mut commands: Commands,
) {
    for (entity, window, persistence) in changed.iter() {
        let new = QuitButtonWindowPersistenceProperty::from(window).into_persistence_property();
        // Avoid change detection if nothing actually changed
        if let Some(old) = persistence
            && *old == new
        {
            continue;
        }

        commands.entity(entity).insert(new);
    }
}

fn handle_persistence_loaded(
    event: On<PersistenceLoaded<QuitButtonWindowPersistenceProperty>>,
    mut windows: Query<&mut Window, With<QuitButtonWindow>>,
    mut commands: Commands,
) {
    if let Ok(mut window) = windows.get_mut(event.entity) {
        info!(
            ?event,
            "Applying loaded persistence data to Quit Button window"
        );
        window.position = event.property.position;
        window.resolution = event.property.resolution.clone();

        // Insert the property so it can be tracked for changes
        commands.entity(event.entity).insert(event.property.clone());
    }
}

fn handle_quit_button(
    mut interaction_query: Query<&Interaction, (Changed<Interaction>, With<QuitButton>)>,
    mut app_exit_events: MessageWriter<AppExit>,
) {
    for interaction in &mut interaction_query {
        if *interaction == Interaction::Pressed {
            app_exit_events.write(AppExit::Success);
        }
    }
}

fn update_quit_button_visuals(
    mut interaction_query: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Button,
            &Children,
        ),
        (Changed<Interaction>, With<QuitButton>),
    >,
    mut text_query: Query<&mut Text>,
) {
    for (interaction, mut background_color, mut border_color, mut button, children) in
        &mut interaction_query
    {
        let mut text = text_query.get_mut(children[0]).unwrap();

        match *interaction {
            Interaction::Pressed => {
                **text = "Quit!".to_string();
                *background_color = BackgroundColor(QUIT_BUTTON_PRESSED);
                *border_color = BorderColor::all(QUIT_BUTTON_BORDER_PRESSED);
                button.set_changed();
            }
            Interaction::Hovered => {
                **text = "Quit?".to_string();
                *background_color = BackgroundColor(QUIT_BUTTON_HOVERED);
                *border_color = BorderColor::all(QUIT_BUTTON_BORDER_HOVERED);
                button.set_changed();
            }
            Interaction::None => {
                **text = "Quit".to_string();
                *background_color = BackgroundColor(QUIT_BUTTON_NORMAL);
                *border_color = BorderColor::all(QUIT_BUTTON_BORDER_NORMAL);
            }
        }
    }
}
