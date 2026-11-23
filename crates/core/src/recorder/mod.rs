//! The recorder: a child process keeping a replay buffer. On Linux it is
//! gpu-screen-recorder; on Windows it is Zenith's own `zenith-rec`, which
//! takes the same command line and prints the same lists. `args` builds its
//! command line, `supervisor` decides what to do with it.

pub mod args;
pub mod outputs;
pub mod supervisor;

pub use args::{Track, command_line, tracks};
pub use supervisor::{Action, Event, State, Supervisor};

/// The recorder's program name: looked for on the path on Linux, beside
/// Zenith's own programs on Windows (`paths::program`).
#[cfg(not(windows))]
pub const PROGRAM: &str = "gpu-screen-recorder";
#[cfg(windows)]
pub const PROGRAM: &str = "zenith-rec";

/// How the recorder says it is recording, from a line of what it prints:
/// `... recording with ddagrab -> h264_nvenc (frames kept on the GPU) at 60
/// fps, ...` gives `ddagrab -> h264_nvenc (frames kept on the GPU)`. Only
/// Zenith's own recorder says so; `None` for any other line.
pub fn way_from_line(line: &str) -> Option<String> {
    let (_, rest) = line.split_once("recording with ")?;
    let way = rest.split_once(" at ").map_or(rest, |(way, _)| way).trim();
    (!way.is_empty()).then(|| way.to_string())
}

/// Whether a way of recording is one of the fallbacks: copying the
/// desktop's pixels, or encoding on the processor. Both work, and both
/// cost the game more than the graphics card's own way.
pub fn is_fallback(way: &str) -> bool {
    ["gdigrab", "libx264", "libx265", "libsvtav1"].iter().any(|slow| way.contains(slow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_way_of_recording_is_read_from_what_the_recorder_says() {
        let line = "2026-11-20 18:21:01.688 recording with ddagrab -> h264_nvenc (frames kept on the GPU) at 60 fps, 20000 kbit/s, a buffer of 60 s";
        assert_eq!(way_from_line(line).as_deref(), Some("ddagrab -> h264_nvenc (frames kept on the GPU)"));
        assert_eq!(way_from_line("ffmpeg version 9"), None);
        assert_eq!(way_from_line("recording with "), None);
    }

    #[test]
    fn copying_pixels_or_encoding_on_the_processor_is_a_fallback() {
        assert!(!is_fallback("ddagrab -> h264_nvenc (frames kept on the GPU)"));
        assert!(!is_fallback("gfxcapture -> h264_amf (frames downloaded)"));
        assert!(is_fallback("gdigrab -> h264_nvenc (frames downloaded)"));
        assert!(is_fallback("ddagrab -> libx264 (frames downloaded)"));
    }
}
