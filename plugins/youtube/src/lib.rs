//! YouTube plugin — a video window with an embedded player plus AI tools
//! (`youtube_search`, `youtube_play`, `youtube_suggest`) that scrape
//! YouTube's public search results and start playback in the YouTube window.

pub mod plugin;
pub mod routes;
pub mod suggest;
pub mod tools;
pub mod youtube_client;

pub use plugin::YoutubePlugin;
