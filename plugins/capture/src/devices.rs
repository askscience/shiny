//! What can be recorded: audio sources (through PulseAudio/PipeWire), cameras,
//! encoders and the screen.

use serde::Serialize;
use std::process::Command;

#[derive(Clone, Debug, Serialize)]
pub struct AudioSource {
    pub name: String,
    pub description: String,
    /// The monitor of an output (what the speakers play).
    pub monitor: bool,
    pub default: bool,
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn default_source() -> Option<String> {
    pactl(&["get-default-source"]).filter(|s| !s.is_empty())
}

/// The monitor of the default output: the desktop's sound.
pub fn default_monitor() -> Option<String> {
    pactl(&["get-default-sink"])
        .filter(|s| !s.is_empty())
        .map(|s| format!("{s}.monitor"))
}

pub fn audio_sources() -> Vec<AudioSource> {
    let def_src = default_source();
    let def_mon = default_monitor();
    let Some(json) = pactl(&["-f", "json", "list", "sources"]) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return Vec::new();
    };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let name = s.get("name")?.as_str()?.to_string();
                    let description = s
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or(&name)
                        .to_string();
                    let monitor = name.ends_with(".monitor")
                        || s.get("monitor_of_sink").is_some_and(|m| m.is_string());
                    let default =
                        Some(&name) == def_src.as_ref() || Some(&name) == def_mon.as_ref();
                    Some(AudioSource {
                        name,
                        description,
                        monitor,
                        default,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A source name from the window: `""`/`"default"` mean the default one of
/// its kind; anything else must be a known source.
pub fn resolve_source(want: Option<&str>, monitor: bool) -> Option<String> {
    let want = want?;
    if want.is_empty() || want == "default" {
        return if monitor {
            default_monitor()
        } else {
            default_source()
        };
    }
    audio_sources()
        .into_iter()
        .find(|s| s.name == want)
        .map(|s| s.name)
}
