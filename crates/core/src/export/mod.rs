//! Turning a clip and its edit into a file: `plan` decides the size, rate
//! and bitrates that fit a target, `ffmpeg` builds the commands that do it.

pub mod ffmpeg;
pub mod plan;

pub use plan::{Plan, PlanError, Preset, Source, plan};
