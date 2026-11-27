//! `HudPlugin`: a bubble over whoever answers in the chat, the notices
//! ("clip saved"), the title, and (F3) the timings and what is drawn.

use bevy::prelude::*;

use crate::bridge::{ClipSaved, Host, HostStatus};
use crate::camera::Direction;
use crate::camera::FollowCamera;
use crate::control::Order;
use crate::telemetry::Latency;

/// Somebody else says something: a passer-by, a car's horn. Shown in a
/// bubble at that place in the world for a moment.
#[derive(Message, Clone, Debug)]
pub struct OtherSaid {
    pub text: String,
    pub at: Vec3,
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<OtherSaid>()
            .add_systems(Startup, setup)
            .add_systems(Update, (draw, toggle_timings, clips, title, photo).chain())
            .add_systems(PostUpdate, other_bubble.after(TransformSystems::Propagate));
    }
}

/// The game's typeface: the engine's own has no accented letters.
#[derive(Resource, Clone)]
pub struct GameFont(pub Handle<Font>);

#[derive(Component)]
struct LatencyLabel;

/// A short notice in the middle of the top edge ("clip saved").
#[derive(Component)]
struct Toast {
    left: f32,
}

/// The game's name, shown as it starts.
#[derive(Component)]
struct TitleCard;

/// Everything of the HUD, to put it away for a photo.
#[derive(Component)]
pub struct HudRoot;

/// The other speaker's bubble: where in the world it hangs.
#[derive(Component)]
struct OtherBubble {
    left: f32,
    at: Vec3,
}

#[derive(Component)]
struct OtherBubbleText;

const INK: Color = Color::srgb(0.20, 0.22, 0.30);
const OTHER_WIDTH: f32 = 240.0;

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
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..default()
            },
        ))
        .with_children(|hud| {
            hud.spawn((Text::new(""), text(13.0), LatencyLabel, Visibility::Hidden));
        });

    // A notice at the top, and the title across the middle.
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            top: Val::Px(18.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(""),
                text(17.0),
                Toast { left: 0.0 },
                Visibility::Hidden,
            ));
        });
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            top: Val::Percent(30.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Happy Walk"),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(72.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 1.0, 1.0)),
                TitleCard,
            ));
        });

    commands
        .spawn((
            OtherBubble {
                left: 0.0,
                at: Vec3::ZERO,
            },
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(OTHER_WIDTH),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 0.98, 0.90, 0.94)),
            Visibility::Hidden,
        ))
        .with_children(|bubble| {
            bubble.spawn((
                Text::new(""),
                text(15.0),
                TextLayout::justify(Justify::Center),
                OtherBubbleText,
            ));
        });
}

fn other_bubble(
    time: Res<Time>,
    mut said: MessageReader<OtherSaid>,
    cameras: Query<(&Camera, &GlobalTransform), With<FollowCamera>>,
    mut bubbles: Query<(&mut OtherBubble, &mut Node, &mut Visibility)>,
    mut texts: Query<&mut Text, With<OtherBubbleText>>,
) {
    let Ok((mut bubble, mut node, mut visibility)) = bubbles.single_mut() else {
        return;
    };
    for line in said.read() {
        if let Ok(mut text) = texts.single_mut() {
            **text = line.text.clone();
        }
        bubble.at = line.at;
        bubble.left = 1.6 + line.text.chars().count() as f32 * 0.05;
    }
    bubble.left = (bubble.left - time.delta_secs()).max(0.0);
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    match camera.world_to_viewport(view, bubble.at) {
        Ok(at) if bubble.left > 0.0 => {
            *visibility = Visibility::Visible;
            node.left = Val::Px(at.x - OTHER_WIDTH / 2.0);
            node.top = Val::Px((at.y - 50.0).max(8.0));
        }
        _ => *visibility = Visibility::Hidden,
    }
}

/// "Clip": ask the page to keep the last seconds, and say how it went.
fn clips(
    time: Res<Time<Real>>,
    status: Res<HostStatus>,
    host: Res<Host>,
    mut orders: MessageReader<Order>,
    mut saved: MessageReader<ClipSaved>,
    mut toasts: Query<(&mut Toast, &mut Text, &mut Visibility)>,
) {
    let Ok((mut toast, mut text, mut visibility)) = toasts.single_mut() else {
        return;
    };
    let italian = status.lang.starts_with("it");
    for order in orders.read() {
        if *order == Order::Clip {
            host.save_clip();
            **text = if italian {
                "Salvo la clip…".into()
            } else {
                "Saving the clip…".into()
            };
            toast.left = 4.0;
        }
    }
    for clip in saved.read() {
        **text = match (clip.ok, italian) {
            (true, true) => format!("Clip salvata ({:.0} s)", clip.seconds),
            (true, false) => format!("Clip saved ({:.0} s)", clip.seconds),
            (false, true) => "Niente clip: la registrazione è spenta".into(),
            (false, false) => "No clip: recording is off".into(),
        };
        toast.left = 3.5;
    }
    toast.left = (toast.left - time.delta_secs()).max(0.0);
    *visibility = if toast.left > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
}

/// The title stays a few seconds, then fades away.
fn title(
    time: Res<Time<Real>>,
    mut cards: Query<(Entity, &mut TextColor), With<TitleCard>>,
    mut commands: Commands,
) {
    const STAYS: f32 = 3.5;
    const FADES: f32 = 1.2;
    let age = time.elapsed_secs();
    for (entity, mut colour) in &mut cards {
        if age > STAYS + FADES {
            commands.entity(entity).despawn();
        } else {
            colour
                .0
                .set_alpha(1.0 - ((age - STAYS) / FADES).clamp(0.0, 1.0));
        }
    }
}

/// Photo mode puts the HUD away.
fn photo(
    direction: Res<Direction>,
    mut roots: Query<&mut Visibility, (With<HudRoot>, Without<Toast>)>,
) {
    if !direction.is_changed() {
        return;
    }
    for mut visibility in &mut roots {
        *visibility = if direction.photo {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
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

fn draw(latency: Res<Latency>, mut timings: Query<&mut Text, With<LatencyLabel>>) {
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
            let (all, seen, triangles) = latency.drawn;
            text.push_str(&format!(
                "\ndrawn  {seen} of {all} meshes, {triangles} triangles"
            ));
        }
    }
}
