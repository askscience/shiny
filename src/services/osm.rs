//! OpenStreetMap client — the implementation lives in the shared `shiny-osm`
//! crate so core and the traveler plugin use exactly the same requests.

pub use shiny_osm::{place_label, GeoPlace, OsmClient as OsmService, RouteResult, RouteStep};
