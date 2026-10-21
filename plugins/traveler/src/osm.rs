//! OpenStreetMap client — the implementation lives in the shared `shiny-osm`
//! crate so the plugin and core use exactly the same requests.

pub use shiny_osm::{
    haversine_km, place_label, GeoPlace, OsmClient, RouteResult, RouteStep,
};
