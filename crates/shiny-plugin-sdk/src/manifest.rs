use serde::{Deserialize, Serialize};

/// Plugin manifest — the `plugin.toml` shipped inside every plugin archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: semver::Version,
    /// Plugin API level the plugin was built against. The loader compares this
    /// to `shiny_plugin_sdk::CORE_API_LEVEL`.
    pub api_level: u32,
    /// Entry point symbol of the cdylib. Web-app plugins (`kind = "web"` in
    /// `plugin.toml`) have no cdylib and may omit it.
    #[serde(default)]
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
    /// Without plugins enabled, core is just the AI assistant; this is the doc
    /// string the admin UI shows as "what this plugin adds".
    #[serde(default)]
    pub summary: Option<String>,
    /// Path inside the plugin archive to the migrations dir (relative to the
    /// plugin install dir).
    #[serde(default = "default_migrations_dir")]
    pub migrations_dir: String,
    #[serde(default = "default_skills_dir")]
    #[allow(dead_code)] // reserved: parsed from plugin.toml, not yet consumed
    pub skills_dir: String,
    #[serde(default = "default_web_dir")]
    pub web_dir: String,
    /// Optional signature: hex-encoded ed25519 signature of `plugin.toml +
    /// the cdylib bytes`. If present, the installer verifies unless
    /// `--insecure` was passed.
    #[serde(default)]
    #[allow(dead_code)] // reserved: verification hook not yet wired up
    pub signature: Option<String>,
}

impl Manifest {
    /// Standard manifest for a first-party cdylib plugin.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: semver::Version::new(0, 1, 0),
            api_level: crate::CORE_API_LEVEL,
            entry_symbol: crate::plugin::PLUGIN_ENTRY_SYMBOL.into(),
            target_triple: None,
            description: None,
            author: Some("shiny".into()),
            summary: None,
            migrations_dir: default_migrations_dir(),
            skills_dir: default_skills_dir(),
            web_dir: default_web_dir(),
            signature: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
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