//! The streaming platforms Capture knows, with their ingest servers and the
//! limits that matter for one encode shared by all of them.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Platform {
    pub id: &'static str,
    pub name: &'static str,
    /// Default ingest server; empty when the dashboard gives one per account.
    pub server: &'static str,
    /// The highest video bitrate the platform accepts (kbit/s).
    pub max_kbps: u32,
    /// What it recommends for 1080p30.
    pub kbps_1080: u32,
    /// Where the user finds the stream key.
    pub key_help: &'static str,
}

pub const PLATFORMS: &[Platform] = &[
    Platform {
        id: "youtube",
        name: "YouTube",
        server: "rtmps://a.rtmp.youtube.com/live2",
        max_kbps: 51_000,
        kbps_1080: 8_000,
        key_help: "https://studio.youtube.com/channel/UC/livestreaming",
    },
    Platform {
        id: "twitch",
        name: "Twitch",
        server: "rtmp://live.twitch.tv/app",
        max_kbps: 6_000,
        kbps_1080: 6_000,
        key_help: "https://dashboard.twitch.tv/settings/stream",
    },
    Platform {
        id: "kick",
        name: "Kick",
        server: "",
        max_kbps: 8_000,
        kbps_1080: 6_000,
        key_help: "https://kick.com/dashboard/settings/stream",
    },
    Platform {
        id: "facebook",
        name: "Facebook Live",
        server: "rtmps://live-api-s.facebook.com:443/rtmp",
        max_kbps: 9_000,
        kbps_1080: 6_000,
        key_help: "https://www.facebook.com/live/producer",
    },
    Platform {
        id: "custom",
        name: "RTMP",
        server: "",
        max_kbps: 51_000,
        kbps_1080: 6_000,
        key_help: "",
    },
];

pub fn get(id: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.id == id)
}

/// `server` + `/` + `key`, for an `rtmp://` or `rtmps://` server.
pub fn ingest_url(server: &str, key: &str) -> Result<String, String> {
    let server = server.trim().trim_end_matches('/');
    if !(server.starts_with("rtmp://") || server.starts_with("rtmps://")) {
        return Err("the server must be an rtmp:// or rtmps:// address".into());
    }
    if server.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the server address has spaces".into());
    }
    let key = key.trim();
    if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the stream key has spaces".into());
    }
    Ok(if key.is_empty() {
        server.to_string()
    } else {
        format!("{server}/{key}")
    })
}

/// One encode feeds every destination, so its bitrate is the lowest limit
/// among them, capped by what the user asked for.
pub fn shared_kbps(wanted: u32, platforms: &[&Platform]) -> u32 {
    platforms
        .iter()
        .map(|p| p.max_kbps)
        .fold(wanted.max(500), u32::min)
}

/// Hide all but the last four characters of a key.
pub fn mask_key(key: &str) -> String {
    let n = key.chars().count();
    if n == 0 {
        return String::new();
    }
    let tail: String = key.chars().skip(n.saturating_sub(4)).collect();
    format!("••••{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_join_and_validate() {
        assert_eq!(
            ingest_url("rtmp://live.twitch.tv/app/", "live_1").unwrap(),
            "rtmp://live.twitch.tv/app/live_1"
        );
        assert!(ingest_url("http://x", "k").is_err());
        assert!(ingest_url("rtmp://x", "a b").is_err());
    }

    #[test]
    fn bitrate_respects_the_strictest_platform() {
        let yt = get("youtube").unwrap();
        let tw = get("twitch").unwrap();
        assert_eq!(shared_kbps(8000, &[yt, tw]), 6000);
        assert_eq!(shared_kbps(4500, &[yt]), 4500);
    }

    #[test]
    fn keys_are_masked() {
        assert_eq!(mask_key("live_123456789"), "••••6789");
        assert_eq!(mask_key(""), "");
    }
}
