//! YouTube plugin — a video window with an embedded player plus two AI tools
//! (`youtube_search`, `youtube_play`) that scrape YouTube's public search
//! results and start playback in the YouTube window.

pub mod plugin;
pub mod tools;
pub mod youtube_client;

pub use plugin::YoutubePlugin;
