use serde::{Deserialize, Serialize};

/// Per-request agent context passed to every tool invocation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentContext {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading: Option<f64>,
    pub lang: String,
    pub ollama_model: Option<String>,
    /// False on a vertical, phone-like screen: the desktop is a single column
    /// there and the workspace tools are inert. Defaults to true so older
    /// clients (and tests) keep the wide-screen behaviour.
    #[serde(default = "workspaces_enabled_default")]
    pub workspaces_enabled: bool,
    /// Plugins the user has focused. Empty = focus mode off and every active
    /// plugin is available. Non-empty = the hard scope for this turn: only
    /// `web_search` and these plugins' own tools run, refused in
    /// `agent_tools::execute_action`. Only the user changes it. Defaults to
    /// empty for older clients.
    #[serde(default)]
    pub focused: Vec<String>,
}

fn workspaces_enabled_default() -> bool {
    true
}