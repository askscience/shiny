//! Cron contributions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSpec {
    /// Cron entry tag — resolved by the plugin's `cron_handler(tag)` when
    /// the scheduler ticks. A plugin can publish multiple crons with
    /// different tags.
    pub tag: String,
    /// "HH:MM" 24-hour local time tick. Hourly resolution.
    pub at: String,
}

/// Handle returned by the plugin's cron factory.
pub type CronEntry = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;