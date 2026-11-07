//! Traveler plugin's tool registry. Returns `Arc<dyn Tool>` ready to be
//! merged into the core registry.

use std::sync::Arc;
use shiny_plugin_sdk::tools::Tool;

pub mod artifacts;
pub mod diary_tools;
pub mod locations;
pub mod maps;
pub mod navigate;
pub mod plan;
pub mod trips;

/// Largest page a listing tool (`list_locations`, `list_diary`,
/// `search_diary`) returns in one call.
pub const MAX_LIST_LIMIT: i64 = 200;

/// The `limit` param of a listing tool, defaulting to `default` and clamped
/// to `1..=MAX_LIST_LIMIT`.
pub fn list_limit(params: &serde_json::Value, default: i64) -> i64 {
    params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
        .clamp(1, MAX_LIST_LIMIT)
}

pub fn all_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        // Trips
        Arc::new(trips::CreateTrip),
        Arc::new(trips::ListTrips),
        Arc::new(trips::GetTrip),
        Arc::new(trips::GetActiveTrip),
        Arc::new(trips::StartTrip),
        Arc::new(trips::EndTrip),
        Arc::new(trips::TripStats),
        // Locations
        Arc::new(locations::SubmitLocation),
        Arc::new(locations::ListLocations),
        Arc::new(locations::TripRoute),
        // Maps
        Arc::new(maps::MapSearch),
        Arc::new(maps::MapReverse),
        Arc::new(maps::MapRoute),
        Arc::new(maps::MapPoi),
        // Navigation
        Arc::new(navigate::NavigateTo),
        // Diary
        Arc::new(diary_tools::ListDiary),
        Arc::new(diary_tools::GetDiary),
        Arc::new(diary_tools::SearchDiary),
        Arc::new(diary_tools::GenerateDiary),
        // Planning
        Arc::new(plan::PlanTrip),
        // Artifact cards
        Arc::new(artifacts::ShowArtifact),
        Arc::new(artifacts::UpdateArtifact),
    ]
}

#[cfg(test)]
mod tests {
    use super::{list_limit, MAX_LIST_LIMIT};
    use serde_json::json;

    #[test]
    fn list_limit_defaults_and_clamps() {
        assert_eq!(list_limit(&json!({}), 20), 20);
        assert_eq!(list_limit(&json!({"limit": 5}), 20), 5);
        assert_eq!(list_limit(&json!({"limit": 100000}), 20), MAX_LIST_LIMIT);
        assert_eq!(list_limit(&json!({"limit": 0}), 20), 1);
        assert_eq!(list_limit(&json!({"limit": -3}), 50), 1);
    }
}
