//! The two Gokapi REST calls `gokapi-cli` has no command for: per-file
//! metadata (download count, expiry) and delete. Both go to the loopback port
//! with the `apikey` header; nothing here crosses the public tunnel.

use std::time::Duration;

use serde::Deserialize;

use super::server::Instance;

/// The subset of Gokapi's `models.FileApiOutput` Shiny reads. Shared with
/// the CLI's `upload --json` output, which prints the same struct.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct GokapiFile {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "UrlDownload")]
    pub url_download: String,
    #[serde(rename = "SizeBytes")]
    pub size_bytes: i64,
    #[serde(rename = "ExpireAt")]
    pub expire_at: i64,
    #[serde(rename = "UnlimitedTime")]
    pub unlimited_time: bool,
    #[serde(rename = "DownloadsRemaining")]
    pub downloads_remaining: i64,
    #[serde(rename = "DownloadCount")]
    pub download_count: i64,
    #[serde(rename = "UnlimitedDownloads")]
    pub unlimited_downloads: bool,
    #[serde(rename = "IsPasswordProtected")]
    pub is_password_protected: bool,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())
}

/// True when the server's API answers on the loopback port.
pub async fn healthy(inst: &Instance) -> bool {
    let Ok(c) = client() else { return false };
    matches!(
        c.get(format!("{}/api/info/version", inst.local_url()))
            .header("apikey", &inst.api_key)
            .send()
            .await,
        Ok(r) if r.status().is_success()
    )
}

/// Metadata for a live file; `Ok(None)` when Gokapi no longer has it
/// (expired, download limit reached, or deleted).
pub async fn file_info(inst: &Instance, id: &str) -> Result<Option<GokapiFile>, String> {
    let resp = client()?
        .get(format!(
            "{}/api/files/list/{}",
            inst.local_url(),
            urlencode(id)
        ))
        .header("apikey", &inst.api_key)
        .send()
        .await
        .map_err(|e| format!("gokapi: {e}"))?;
    match resp.status().as_u16() {
        404 => Ok(None),
        s if (200..300).contains(&s) => resp
            .json::<GokapiFile>()
            .await
            .map(Some)
            .map_err(|e| format!("gokapi: bad file info: {e}")),
        s => Err(format!("gokapi: HTTP {s}")),
    }
}

/// Delete a file. Already-gone files count as success.
pub async fn delete(inst: &Instance, id: &str) -> Result<(), String> {
    let resp = client()?
        .delete(format!("{}/api/files/delete", inst.local_url()))
        .header("apikey", &inst.api_key)
        .header("id", id)
        .send()
        .await
        .map_err(|e| format!("gokapi: {e}"))?;
    match resp.status().as_u16() {
        s if (200..300).contains(&s) || s == 404 => Ok(()),
        s => Err(format!("gokapi: delete failed (HTTP {s})")),
    }
}

/// Gokapi ids are alphanumeric; encode anyway so a stored value can never
/// change the request path.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_api_output() {
        let raw = r#"{"Id":"abc","Name":"a.txt","Size":"1 kB","UrlDownload":"https://h/d?id=abc",
            "SizeBytes":1024,"ExpireAt":1700000000,"UnlimitedTime":false,"DownloadsRemaining":3,
            "DownloadCount":2,"UnlimitedDownloads":false,"IsPasswordProtected":true,"Extra":1}"#;
        let f: GokapiFile = serde_json::from_str(raw).unwrap();
        assert_eq!(f.id, "abc");
        assert_eq!(f.url_download, "https://h/d?id=abc");
        assert_eq!(f.downloads_remaining, 3);
        assert!(f.is_password_protected);
    }

    #[test]
    fn urlencode_escapes_path_characters() {
        assert_eq!(urlencode("aZ9-_"), "aZ9-_");
        assert_eq!(urlencode("../x"), "%2E%2E%2Fx");
    }
}
