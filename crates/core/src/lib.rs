//! Zenith's logic: settings, the recorder's command line and state,
//! game detection, the clip library, edits and export planning. Nothing here
//! links a UI toolkit, so every test runs in a plain `cargo test`.

pub mod autostart;
pub mod config;
pub mod doctor;
pub mod edit;
pub mod export;
pub mod games;
pub mod hotkey;
pub mod install;
pub mod ipc;
pub mod applog;
pub mod jobs;
pub mod library;
pub mod media;
pub mod paths;
pub mod recorder;
pub mod report;
pub mod update;
pub mod volumes;
