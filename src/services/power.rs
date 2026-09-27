//! Host power control for the top-bar power menu — reboot, power off and
//! suspend, through the freedesktop **logind** D-Bus interface
//! (`org.freedesktop.login1.Manager`).
//!
//! Like `network.rs`, `audio.rs` and `bluetooth.rs`, this belongs to the
//! machine rather than a traveler: it is core chrome (a sibling of
//! `hudNetwork.js`) and follows the same contract — probe, degrade quietly when
//! logind is absent, and let the API layer keep mutations loopback-only.
//!
//! Reads report which actions the current session may perform
//! (`CanReboot` / `CanPowerOff` / `CanSuspend`), so the menu can disable an
//! entry the session would refuse. Mutations call the matching logind method
//! with `interactive = false`: the menu confirms the destructive ones itself,
//! and a kiosk has no console to prompt on.

use serde::Serialize;

/// Failed logind probe, shown by the menu instead of the actions.
#[cfg(target_os = "linux")]
const UNAVAILABLE: &str = "logind is not reachable";
#[cfg(not(target_os = "linux"))]
const UNAVAILABLE: &str = "power control requires Linux (logind)";

/// Which power actions the current session allows.
#[derive(Clone, Debug, Serialize)]
pub struct PowerStatus {
    pub available: bool,
    pub reason: Option<String>,
    pub can_reboot: bool,
    pub can_power_off: bool,
    pub can_suspend: bool,
}

impl PowerStatus {
    fn unavailable(reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            can_reboot: false,
            can_power_off: false,
            can_suspend: false,
        }
    }
}

/// Interpret a logind `Can*` reply. `yes` means it will just work; `challenge`
/// means a polkit prompt may appear, which the local session satisfies. `no`
/// and `na` (not available) are both treated as not offered.
pub fn capability_allowed(reply: &str) -> bool {
    matches!(reply.trim(), "yes" | "challenge")
}

/// Stateless: every call opens the system bus, so there is nothing to start or
/// cache, and capabilities are read when the menu opens.
#[derive(Clone, Default)]
pub struct PowerService;

impl PowerService {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/* ── Linux: logind over the system D-Bus ───────────────────── */

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
    fn can_power_off(&self) -> zbus::Result<String>;
    fn can_reboot(&self) -> zbus::Result<String>;
    fn can_suspend(&self) -> zbus::Result<String>;
}

#[cfg(target_os = "linux")]
impl PowerService {
    /// Which actions this session allows; `available:false` when logind is
    /// unreachable, in which case the menu shows the reason instead.
    pub async fn status(&self) -> PowerStatus {
        match query_status().await {
            Ok(status) => status,
            Err(err) => PowerStatus::unavailable(&err),
        }
    }

    pub async fn reboot(&self) -> Result<(), String> {
        let connection = system_bus().await?;
        let proxy = Login1ManagerProxy::new(&connection)
            .await
            .map_err(unreachable)?;
        proxy.reboot(false).await.map_err(friendly)
    }

    pub async fn power_off(&self) -> Result<(), String> {
        let connection = system_bus().await?;
        let proxy = Login1ManagerProxy::new(&connection)
            .await
            .map_err(unreachable)?;
        proxy.power_off(false).await.map_err(friendly)
    }

    pub async fn suspend(&self) -> Result<(), String> {
        let connection = system_bus().await?;
        let proxy = Login1ManagerProxy::new(&connection)
            .await
            .map_err(unreachable)?;
        proxy.suspend(false).await.map_err(friendly)
    }
}

#[cfg(target_os = "linux")]
async fn system_bus() -> Result<zbus::Connection, String> {
    zbus::Connection::system().await.map_err(unreachable)
}

#[cfg(target_os = "linux")]
async fn query_status() -> Result<PowerStatus, String> {
    let connection = system_bus().await?;
    let proxy = Login1ManagerProxy::new(&connection)
        .await
        .map_err(unreachable)?;
    let can_reboot = proxy.can_reboot().await.map_err(friendly)?;
    let can_power_off = proxy.can_power_off().await.map_err(friendly)?;
    let can_suspend = proxy.can_suspend().await.map_err(friendly)?;
    Ok(PowerStatus {
        available: true,
        reason: None,
        can_reboot: capability_allowed(&can_reboot),
        can_power_off: capability_allowed(&can_power_off),
        can_suspend: capability_allowed(&can_suspend),
    })
}

/// Opening the bus or a proxy failed: logind is not there.
#[cfg(target_os = "linux")]
fn unreachable(err: impl std::fmt::Display) -> String {
    format!("{UNAVAILABLE} ({err})")
}

/// Turn logind's terse D-Bus errors into something a person can read.
#[cfg(target_os = "linux")]
fn friendly(err: impl std::fmt::Display) -> String {
    let message = err.to_string();
    if message.contains("Interactive authentication required") || message.contains("Access denied") {
        "permission denied — this session may not change the power state".to_string()
    } else if message.contains("Sleep verb not supported") || message.contains("Not supported") {
        "this machine does not support that action".to_string()
    } else if message.contains("Operation inhibited") || message.contains("blocked") {
        "another session is blocking that action".to_string()
    } else {
        message
    }
}

/* ── Not Linux: inert stub ─────────────────────────────────── */

#[cfg(not(target_os = "linux"))]
impl PowerService {
    pub async fn status(&self) -> PowerStatus {
        PowerStatus::unavailable(UNAVAILABLE)
    }

    pub async fn reboot(&self) -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }

    pub async fn power_off(&self) -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }

    pub async fn suspend(&self) -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{capability_allowed, PowerService};

    #[test]
    fn capabilities_match_logind_replies() {
        assert!(capability_allowed("yes"));
        assert!(capability_allowed("challenge"));
        assert!(capability_allowed(" yes "));
        assert!(!capability_allowed("no"));
        assert!(!capability_allowed("na"));
        assert!(!capability_allowed(""));
    }

    #[test]
    fn new_service_is_constructible() {
        // The service is stateless; construction must never touch D-Bus.
        let _ = PowerService::new();
    }
}
