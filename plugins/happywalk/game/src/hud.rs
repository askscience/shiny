//! `UiPlugin`: the speech bubble over the character, the "I am listening"
//! mark, what was understood, and (F3) the timings of the voice chain.

use bevy::prelude::*;

use crate::brain::Said;
use crate::bridge::{Heard, HostStatus};
use crate::camera::FollowCamera;
use crate::player::Player;
use crate::telemetry::Latency;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastHeard>()
            .add_systems(Startup, setup)
            .add_systems(Update, (remember, draw, toggle_timings).chain())
            .add_systems(PostUpdate, bubble.after(TransformSystems::Propagate));
    }
}

/// The latest thing the recogniser understood.
#[derive(Resource, Default)]
pub struct LastHeard {
    pub text: String,
    pub is_final: bool,
}

/// The game's typeface: the engine's own has no accented letters.
#[derive(Resource, Clone)]
pub struct GameFont(pub Handle<Font>);

#[derive(Component)]
struct ListeningLabel;

#[derive(Component)]
struct HeardLabel;

#[derive(Component)]
struct LatencyLabel;

#[derive(Component)]
struct Bubble {
    /// Seconds the line stays up.
    left: f32,
}

#[derive(Component)]
struct BubbleText;

const INK: Color = Color::srgb(0.20, 0.22, 0.30);
const BUBBLE_WIDTH: f32 = 340.0;

fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../assets/DejaVuSans-Bold.ttf").to_vec(),
    ));
    commands.insert_resource(GameFont(font.clone()));
    let text = |size: f32| {
        (
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(INK),
        )
    };

    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            top: Val::Px(16.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|hud| {
            hud.spawn((Text::new(""), text(16.0), ListeningLabel));
            hud.spawn((Text::new(""), text(30.0), HeardLabel));
            hud.spawn((Text::new(""), text(13.0), LatencyLabel, Visibility::Hidden));
        });

    commands
        .spawn((
            Bubble { left: 0.0 },
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(BUBBLE_WIDTH),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(Val::Px(16.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.94)),
            Visibility::Hidden,
        ))
        .with_children(|bubble| {
            bubble.spawn((
                Text::new(""),
                text(18.0),
                TextLayout::justify(Justify::Center),
                BubbleText,
            ));
        });
}

fn remember(mut heard: MessageReader<Heard>, mut last: ResMut<LastHeard>) {
    for h in heard.read() {
        if h.text.is_empty() {
            continue;
        }
        last.text = h.text.clone();
        last.is_final = h.is_final;
    }
}

fn toggle_timings(
    keys: Res<ButtonInput<KeyCode>>,
    mut panel: Query<&mut Visibility, With<LatencyLabel>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        for mut visibility in &mut panel {
            visibility.toggle_visible_hidden();
        }
    }
}

fn draw(
    status: Res<HostStatus>,
    last: Res<LastHeard>,
    latency: Res<Latency>,
    mut listening: Query<
        &mut Text,
        (
            With<ListeningLabel>,
            Without<HeardLabel>,
            Without<LatencyLabel>,
        ),
    >,
    mut heard: Query<
        &mut Text,
        (
            With<HeardLabel>,
            Without<ListeningLabel>,
            Without<LatencyLabel>,
        ),
    >,
    mut timings: Query<
        &mut Text,
        (
            With<LatencyLabel>,
            Without<ListeningLabel>,
            Without<HeardLabel>,
        ),
    >,
) {
    if status.is_changed() {
        if let Ok(mut text) = listening.single_mut() {
            **text = match (status.voice.as_str(), status.speaking) {
                ("listening", true) => format!("((•)) {}", status.lang),
                ("listening", false) => format!("(•) {}", status.lang),
                (other, _) => format!("{other} {} {}", status.lang, status.detail)
                    .trim()
                    .to_string(),
            };
        }
    }
    if last.is_changed() {
        if let Ok(mut text) = heard.single_mut() {
            **text = match (last.text.is_empty(), last.is_final) {
                (true, _) => String::new(),
                (false, true) => last.text.clone(),
                (false, false) => format!("{}…", last.text),
            };
        }
    }
    if latency.is_changed() {
        if let Ok(mut text) = timings.single_mut() {
            **text = latency
                .iter()
                .map(|(link, stat)| {
                    format!(
                        "{link}  {:.0} ms  (mean {:.0}, worst {:.0}, n {})",
                        stat.last,
                        stat.mean(),
                        stat.worst,
                        stat.count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
}

/// Show what the character says, over its head, for a few seconds.
fn bubble(
    time: Res<Time>,
    mut said: MessageReader<Said>,
    players: Query<&GlobalTransform, With<Player>>,
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    mut bubbles: Query<(&mut Bubble, &mut Node, &mut Visibility)>,
    mut texts: Query<&mut Text, With<BubbleText>>,
) {
    let Ok((mut bubble, mut node, mut visibility)) = bubbles.single_mut() else {
        return;
    };
    for line in said.read() {
        if let Ok(mut text) = texts.single_mut() {
            **text = line.text.clone();
        }
        // Long enough to read: a base, and more for a longer line.
        bubble.left = 2.2 + line.text.chars().count() as f32 * 0.05;
    }
    bubble.left = (bubble.left - time.delta_secs()).max(0.0);
    let (Ok(player), Ok((camera, view))) = (players.single(), cameras.single()) else {
        return;
    };
    let above = player.translation() + Vec3::Y * 2.3;
    match camera.world_to_viewport(view, above) {
        Ok(at) if bubble.left > 0.0 => {
            *visibility = Visibility::Visible;
            node.left = Val::Px(at.x - BUBBLE_WIDTH / 2.0);
            node.bottom = Val::Auto;
            node.top = Val::Px((at.y - 70.0).max(8.0));
        }
        _ => *visibility = Visibility::Hidden,
    }
}
