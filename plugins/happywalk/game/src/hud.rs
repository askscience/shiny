//! The HUD: whether the game is listening, what it understood, and how long
//! each link of the voice chain took.

use bevy::prelude::*;

use crate::bridge::{Heard, HostStatus};
use crate::telemetry::Latency;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LastHeard>()
            .add_systems(Startup, setup)
            .add_systems(Update, (remember, draw).chain());
    }
}

/// The latest thing the recogniser understood.
#[derive(Resource, Default)]
pub struct LastHeard {
    pub text: String,
    pub is_final: bool,
}

#[derive(Component)]
struct ListeningLabel;

#[derive(Component)]
struct HeardLabel;

#[derive(Component)]
struct LatencyLabel;

const INK: Color = Color::srgb(0.20, 0.22, 0.30);

fn setup(mut commands: Commands) {
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
            hud.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(INK),
                ListeningLabel,
            ));
            hud.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(30.0),
                    ..default()
                },
                TextColor(INK),
                HeardLabel,
            ));
            hud.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(INK),
                LatencyLabel,
            ));
        });
}

fn remember(mut heard: MessageReader<Heard>, mut last: ResMut<LastHeard>) {
    for h in heard.read() {
        last.text = h.text.clone();
        last.is_final = h.is_final;
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
            let mark = match (status.voice.as_str(), status.speaking) {
                // Plain characters: the built-in font has little else.
                ("listening", true) => "(( )) ",
                ("listening", false) => "( ) ",
                _ => "",
            };
            **text = format!("{mark}{} {} {}", status.voice, status.lang, status.detail)
                .trim()
                .to_string();
        }
    }
    if last.is_changed() {
        if let Ok(mut text) = heard.single_mut() {
            **text = if last.is_final {
                last.text.clone()
            } else {
                format!("{}...", last.text)
            };
        }
    }
    if latency.is_changed() {
        if let Ok(mut text) = timings.single_mut() {
            **text = latency
                .iter()
                .map(|(link, stat)| {
                    format!(
                        "{}  {:.0} ms  (mean {:.0}, worst {:.0}, n {})",
                        link.replace('→', " > "),
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
