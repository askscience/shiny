use serde::{Deserialize, Serialize};

/// Plugin manifest — the `plugin.toml` shipped inside every plugin archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: semver::Version,
    /// Plugin API level the plugin was built against. The loader compares this
    /// to `shiny_plugin_sdk::CORE_API_LEVEL`.
    pub api_level: u32,
    pub entry_symbol: String,
    /// Target triple this binary was compiled for (`x86_64-unknown-linux-gnu`).
    /// The installer refuses archives whose triple doesn't match the host.
    #[serde(default)]
    pub target_triple: Option<String>,
    /// One-line human description.
    #[serde(default)]
    pub description: Option<String>,
    /// Author / publisher.
    #[serde(default)]
    pub author: Option<String>,
    /// Without plugins enabled, core is just the AI sphere; this is the doc
    /// string the admin UI shows as "what this plugin adds".
    #[serde(default)]
    pub summary: Option<String>,
    /// Path inside the plugin archive to the migrations dir (relative to the
    /// plugin install dir).
    #[serde(default = "default_migrations_dir")]
    pub migrations_dir: String,
    #[serde(default = "default_skills_dir")]
    pub skills_dir: String,
    #[serde(default = "default_web_dir")]
    pub web_dir: String,
    /// Optional signature: hex-encoded ed25519 signature of `plugin.toml +
    /// the cdylib bytes`. If present, the installer verifies unless
    /// `--insecure` was passed.
    #[serde(default)]
    pub signature: Option<String>,
}

fn default_migrations_dir() -> String {
    "migrations".into()
}
fn default_skills_dir() -> String {
    "skills".into()
}
fn default_web_dir() -> String {
    "web".into()
}