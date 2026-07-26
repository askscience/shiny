//! Traveler plugin's tool registry. Returns a list of `Arc<dyn Tool>` ready
//! to be merged into the core registry.

use std::sync::Arc;
use shiny_plugin_sdk::tools::Tool;

mod create_trip;
mod list_trips;
mod get_active_trip;
mod map_search;
mod navigate_to;
mod plan_trip;

pub fn all_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(create_trip::CreateTrip),
        Arc::new(list_trips::ListTrips),
        Arc::new(get_active_trip::GetActiveTrip),
        Arc::new(map_search::MapSearch),
        Arc::new(navigate_to::NavigateTo),
        Arc::new(plan_trip::PlanTrip),
    ]
}