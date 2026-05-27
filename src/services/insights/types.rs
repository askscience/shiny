use serde::{Deserialize, Serialize};

/// A small contextual card shown after the user picks a destination (weather, events, places).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightCard {
    pub id: String,
    /// `weather` | `event` | `place`
    pub kind: String,
    pub title: String,
    pub body: String,
    /// Icon stem → `/icons/insights/{icon}.svg` (e.g. `weather-rain`, `place-theatre`)
    pub icon: String,
}
