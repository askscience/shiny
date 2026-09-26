pub mod ai;
pub mod audio;
pub mod ollama;
pub mod openai;
pub mod web_search;
pub mod gpsd;
pub mod network;
pub mod osm;
pub mod diary_gen;
pub mod supertonic;
pub mod whisper;
pub mod artifacts;
pub mod agent_tools;
pub mod insights;
pub mod navigation;
pub mod agent_steps;
pub mod agent_runner;
pub mod agent_cancel;
pub mod chat_memory;
pub mod display;
pub mod touchbar;
pub mod keyboard_backlight;
pub mod screen_brightness;
pub mod unix_user;
pub mod auth_helper;
pub mod iroh_remote;

/// Shared sysfs backlight read/write logic (keyboard + screen).
mod backlight;
