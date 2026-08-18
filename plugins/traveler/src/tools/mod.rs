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
