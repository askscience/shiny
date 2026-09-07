//! Unified notifications (GNOME-style).
//!
//! A plugin can ask the core to show a desktop notification banner either
//! from its web surface (the frontend `notify()` helper) or from a tool —
//! attach a [`Notification`] to an [`ActionOutcome`] with
//! [`ActionOutcome::with_notification`] and the core's frontend will render it.
//!
//! The value is carried inside the outcome's `data` under the reserved
//! `notification` key, so it needs no change to the `ActionOutcome` ABI.

use serde::{Deserialize, Serialize};

fn default_urgency() -> String {
    "normal".into()
}

/// One button shown at the bottom of a notification banner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    /// Button label.
    pub label: String,
    /// Key dispatched back to the frontend as an `app:notification-action`
    /// event (with the notification id) when the button is pressed.
    pub action: String,
}

impl NotificationAction {
    pub fn new(label: impl Into<String>, action: impl Into<String>) -> Self {
        Self { label: label.into(), action: action.into() }
    }
}

/// A desktop notification, modeled on GNOME's banner: an optional title
/// (summary), a body, an urgency level, an icon, and optional action buttons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Short headline (GNOME "summary"). Optional — body-only is allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The main text.
    pub body: String,
    /// `"low" | "normal" | "critical"`. Critical banners stay until dismissed.
    #[serde(default = "default_urgency")]
    pub urgency: String,
    /// Theme icon path (e.g. `"ui/mail"`, `"ui/warning"`). Mutually exclusive
    /// with `plugin`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Plugin name whose `web/icon.svg` should be shown (e.g. `"radio"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// App label shown above the title (defaults to the plugin name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Buttons shown at the bottom of the banner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NotificationAction>,
    /// Auto-dismiss timeout in milliseconds (`0` = stay until dismissed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl Notification {
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            title: None,
            body: body.into(),
            urgency: default_urgency(),
            icon: None,
            plugin: None,
            app: None,
            actions: Vec::new(),
            timeout: None,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn urgency(mut self, urgency: impl Into<String>) -> Self {
        self.urgency = urgency.into();
        self
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn plugin(mut self, plugin: impl Into<String>) -> Self {
        self.plugin = Some(plugin.into());
        self
    }

    pub fn app(mut self, app: impl Into<String>) -> Self {
        self.app = Some(app.into());
        self
    }

    pub fn action(mut self, label: impl Into<String>, action: impl Into<String>) -> Self {
        self.actions.push(NotificationAction::new(label, action));
        self
    }

    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout = Some(ms);
        self
    }
}
