use serde::{Deserialize, Serialize};

/// Per-request agent context passed to every tool invocation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentContext {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub heading: Option<f64>,
    pub lang: String,
    pub ollama_model: Option<String>,
}