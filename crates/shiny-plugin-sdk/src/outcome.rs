use serde_json::Value;
use crate::artifacts::Artifact;
use crate::navigation::NavigationSession;
use crate::notification::Notification;

/// What a tool returns to the agent runner.
#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub action: String,
    pub result: String,
    pub data: Value,
    pub artifact: Option<Artifact>,
    pub extra_artifacts: Vec<Artifact>,
    /// Optional navigator session — set by tools that drive turn-by-turn UI.
    /// Generic replacement for the old hardcoded `navigate_to` extraction.
    pub navigation: Option<NavigationSession>,
}

impl ActionOutcome {
    pub fn ok(action: impl Into<String>, data: Value) -> Self {
        Self {
            action: action.into(),
            result: "ok".into(),
            data,
            artifact: None,
            extra_artifacts: Vec::new(),
            navigation: None,
        }
    }

    pub fn error(action: impl Into<String>, msg: impl Into<String>) -> Self {
        let m: String = msg.into();
        let data = serde_json::json!({ "error": m });
        Self {
            action: action.into(),
            result: "error".into(),
            data,
            artifact: None,
            extra_artifacts: Vec::new(),
            navigation: None,
        }
    }

    pub fn with_artifact(mut self, art: Artifact) -> Self {
        self.artifact = Some(art);
        self
    }

    pub fn with_extra_artifacts(mut self, arts: Vec<Artifact>) -> Self {
        self.extra_artifacts = arts;
        self
    }

    pub fn with_navigation(mut self, nav: NavigationSession) -> Self {
        self.navigation = Some(nav);
        self
    }

    /// Attach a GNOME-style desktop notification. It is carried inside
    /// `data["notification"]` so the core frontend can render it without any
    /// change to this struct's ABI.
    pub fn with_notification(mut self, notif: Notification) -> Self {
        if let Ok(value) = serde_json::to_value(&notif) {
            if let Some(map) = self.data.as_object_mut() {
                map.insert("notification".into(), value);
            }
        }
        self
    }
}