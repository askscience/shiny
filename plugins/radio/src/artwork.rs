//! Now-playing artwork: the track's release on MusicBrainz, its front cover
//! from the Cover Art Archive. Done here rather than in the window because
//! MusicBrainz asks every client to identify itself with a User-Agent, which
//! a browser page cannot set; results are cached for the session.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use shiny_plugin_sdk::services::percent_encode;

/// "Artist — Title" / "Artist - Title" → (artist, title); a title without
/// a separator has no artist.
pub fn split_track_title(raw: &str) -> (Option<String>, String) {
    let raw = raw.trim();
    for sep in [" — ", " – ", " - "] {
        if let Some((artist, title)) = raw.split_once(sep) {
            let (artist, title) = (artist.trim(), title.trim());
            if !artist.is_empty() && !title.is_empty() {
                return (Some(artist.to_string()), title.to_string());
            }
        }
    }
    (None, raw.to_string())
}

/// The MusicBrainz search for a recording, quoted as Lucene phrases.
pub fn recording_query(artist: Option<&str>, title: &str) -> String {
    let q = |s: &str| s.replace(['\\', '"'], " ");
    match artist {
        Some(a) => format!("recording:\"{}\" AND artist:\"{}\"", q(title), q(a)),
        None => format!("recording:\"{}\"", q(title)),
    }
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Shiny/0.1 (https://github.com/askscience/shiny)")
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default()
    })
}

fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn get_json(url: &str) -> Option<Value> {
    let resp = client().get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

/// The front cover's URL for a track, or None (also remembered: a track
/// without a cover is not looked up again).
pub async fn cover_for(artist: Option<&str>, title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let key = format!("{}|{}", artist.unwrap_or("").to_lowercase(), title.to_lowercase());
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return hit;
    }
    let found = async {
        let q = recording_query(artist, title);
        let mb = get_json(&format!(
            "https://musicbrainz.org/ws/2/recording/?query={}&fmt=json&limit=1",
            percent_encode(&q)
        ))
        .await?;
        let release = mb["recordings"][0]["releases"][0]["id"].as_str()?.to_string();
        let caa = get_json(&format!("https://coverartarchive.org/release/{}", percent_encode(&release))).await?;
        let images = caa["images"].as_array()?;
        let front = images.iter().find(|i| i["front"].as_bool() == Some(true)).or(images.first())?;
        front["thumbnails"]["large"].as_str().or(front["image"].as_str()).map(str::to_string)
    }
    .await;
    if let Ok(mut c) = cache().lock() {
        if c.len() > 256 {
            c.clear();
        }
        c.insert(key, found.clone());
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_split_into_artist_and_track() {
        assert_eq!(split_track_title("Daft Punk — One More Time"), (Some("Daft Punk".into()), "One More Time".into()));
        assert_eq!(split_track_title("Daft Punk - One More Time"), (Some("Daft Punk".into()), "One More Time".into()));
        assert_eq!(split_track_title("A-ha – Take On Me"), (Some("A-ha".into()), "Take On Me".into()));
        assert_eq!(split_track_title("Station jingle"), (None, "Station jingle".into()));
        assert_eq!(split_track_title(" - "), (None, "-".into()));
    }

    #[test]
    fn queries_are_quoted_safely() {
        assert_eq!(recording_query(Some("AC/DC"), "T.N.T."), "recording:\"T.N.T.\" AND artist:\"AC/DC\"");
        assert_eq!(recording_query(None, "Say \"hi\""), "recording:\"Say  hi \"");
    }
}
