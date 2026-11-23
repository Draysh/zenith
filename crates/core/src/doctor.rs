//! Is everything in place to clip? Each thing Zenith leans on is
//! looked at, and what is wrong comes with what to do about it. `gather`
//! does the looking; `assess` is the judgement, with no I/O.

use crate::config::Config;
use crate::ipc::{self, Request, Response};
use crate::recorder::outputs::{self, Output};
use serde::Serialize;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    /// Clipping works, with something missing.
    Warning,
    /// Clipping does not work until this is fixed.
    Problem,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub level: Level,
    /// What was found.
    pub detail: String,
    /// What to do, when something is wrong.
    pub fix: String,
}

/// What was found on this machine.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub recorder_installed: bool,
    pub codecs: Vec<String>,
    pub monitors: Vec<Output>,
    pub app_audio: bool,
    pub ffmpeg: bool,
    pub ffprobe: bool,
    pub x264: bool,
    pub sound_player: bool,
    pub folder_writable: bool,
    /// The daemon's state, when it answers.
    pub daemon: Option<(String, Option<String>)>,
}

fn runs(program: &str, arg: &str) -> Option<String> {
    let out = crate::paths::command(program).arg(arg).stdin(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".zenith-clips-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Look at this machine.
pub fn gather(cfg: &Config, recorder: &str, socket: &Path) -> Facts {
    let info = runs(recorder, "--info");
    let encoders = runs("ffmpeg", "-encoders");
    Facts {
        recorder_installed: info.is_some(),
        codecs: info.as_deref().map(outputs::parse_codecs).unwrap_or_default(),
        monitors: outputs::list(recorder),
        app_audio: info.as_deref().is_some_and(|i| i.lines().any(|l| l.trim() == "supports_app_audio|yes")),
        ffmpeg: encoders.is_some(),
        ffprobe: runs("ffprobe", "-version").is_some(),
        x264: encoders.is_some_and(|e| e.contains("libx264")),
        // Only Linux plays the save sound through a program.
        sound_player: !cfg!(windows) && ["pw-play", "paplay"].iter().any(|p| runs(p, "--version").is_some()),
        folder_writable: writable(&cfg.library.dir),
        daemon: match ipc::ask(socket, &Request::Status, Duration::from_secs(1)) {
            Ok(Response::Status(s)) => Some((s.state, s.detail)),
            _ => None,
        },
    }
}

/// A codec's name as people write it.
fn codec_name(codec: &str) -> &str {
    match codec {
        "h264" => "H.264",
        "hevc" => "HEVC",
        "av1" => "AV1",
        other => other,
    }
}

/// The system the checks are worded for: what is missing is put right in
/// another way on each (a package to install, or Zenith to reinstall).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
}

impl Platform {
    /// The one this program was built for.
    pub fn current() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Linux }
    }

    /// The recorder's program there.
    fn recorder(self) -> &'static str {
        match self {
            Self::Linux => "gpu-screen-recorder",
            Self::Windows => "zenith-rec",
        }
    }
}

/// Judge what was found, for these settings, on this system.
pub fn assess(facts: &Facts, cfg: &Config) -> Vec<Check> {
    assess_on(facts, cfg, Platform::current())
}

/// Judge what was found, for these settings, in the words of `platform`.
pub fn assess_on(facts: &Facts, cfg: &Config, platform: Platform) -> Vec<Check> {
    let check = |name, level, detail: String, fix: &str| Check { name, level, detail, fix: if level == Level::Ok { String::new() } else { fix.into() } };
    let windows = platform == Platform::Windows;
    let recorder = platform.recorder();
    let mut out = Vec::new();

    out.push(if facts.recorder_installed {
        check("Recorder", Level::Ok, format!("{recorder} is installed."), "")
    } else {
        check(
            "Recorder",
            Level::Problem,
            format!("{recorder} was not found."),
            // On Windows the recorder is a part of Zenith, not a package.
            if windows { "Reinstall Zenith: its recorder is missing." } else { "Install the gpu-screen-recorder package from your distribution." },
        )
    });

    if facts.recorder_installed {
        let codec = cfg.capture.codec.as_str();
        let gpu: Vec<&str> =
            ["h264", "hevc", "av1"].into_iter().filter(|c| facts.codecs.iter().any(|f| f == c)).map(codec_name).collect();
        out.push(if facts.codecs.iter().any(|c| c == codec) {
            check("Video encoding", Level::Ok, format!("Your GPU records {}.", gpu.join(", ")), "")
        } else if gpu.is_empty() {
            check(
                "Video encoding",
                Level::Problem,
                "Your GPU offers no video encoder.".into(),
                if windows {
                    "Update your graphics driver."
                } else {
                    "Install your GPU's video drivers (Mesa VA-API drivers for AMD and Intel, the NVIDIA driver for NVIDIA)."
                },
            )
        } else {
            check(
                "Video encoding",
                Level::Problem,
                format!("Your GPU cannot record {}; it can record {}.", codec_name(codec), gpu.join(", ")),
                "Pick another format in Settings.",
            )
        });

        let source = cfg.capture.source.as_str();
        out.push(if facts.monitors.is_empty() {
            check(
                "Screen capture",
                Level::Problem,
                "The recorder found no monitor it can capture.".into(),
                &format!("Run `{recorder} --list-capture-options` in a terminal to see why."),
            )
        } else if source != "screen" && source != "portal" && !facts.monitors.iter().any(|m| m.name == source) {
            check(
                "Screen capture",
                Level::Problem,
                format!("The monitor {source} set to be recorded is not connected."),
                "Pick a monitor in Settings.",
            )
        } else {
            let (name, size) = outputs::resolve(source, &facts.monitors);
            check("Screen capture", Level::Ok, format!("Recording {name} at {}\u{00D7}{}.", size.0, size.1), "")
        });

        if cfg.audio.chat {
            out.push(if facts.app_audio {
                check("Sound tracks", Level::Ok, "Game, voice chat and microphone can each have a track.".into(), "")
            } else {
                check(
                    "Sound tracks",
                    Level::Warning,
                    "Voice chat cannot be kept apart from the game sound here.".into(),
                    if windows {
                        "This version of Windows cannot record one application apart from the rest. Turn Voice chat off in Settings."
                    } else {
                        "This needs PipeWire as the sound system. Until then, turn Voice chat off in Settings."
                    },
                )
            });
        }
    }

    out.push(match (facts.ffmpeg && facts.ffprobe, facts.x264) {
        (true, true) => check("Editing and export", Level::Ok, "ffmpeg is installed, with the H.264 encoder.".into(), ""),
        (true, false) => check(
            "Editing and export",
            Level::Problem,
            "ffmpeg is installed without libx264, which exports need.".into(),
            if windows { "Reinstall Zenith: it comes with a full build of ffmpeg." } else { "Install a full build of ffmpeg." },
        ),
        _ => check(
            "Editing and export",
            Level::Problem,
            "ffmpeg was not found. Clips cannot be filed, shown or exported without it.".into(),
            if windows { "Reinstall Zenith: it comes with ffmpeg." } else { "Install the ffmpeg package." },
        ),
    });

    out.push(if facts.folder_writable {
        check("Clip folder", Level::Ok, format!("{} can be written to.", cfg.library.dir.display()), "")
    } else {
        check(
            "Clip folder",
            Level::Problem,
            format!("{} cannot be written to.", cfg.library.dir.display()),
            "Pick another folder in Settings, or fix its permissions.",
        )
    });

    out.push(match &facts.daemon {
        Some((state, detail)) if state == "failed" => check(
            "Background recorder",
            Level::Problem,
            format!("It is running, but capture failed: {}.", detail.as_deref().unwrap_or("no reason given")),
            if windows {
                // There is no tray icon there: the app has the switch.
                "Fix what is listed above, then switch capture on again in Zenith."
            } else {
                "Fix what is listed above, then switch capture on again from the tray icon."
            },
        ),
        Some(_) => check("Background recorder", Level::Ok, "It is running.".into(), ""),
        None => check(
            "Background recorder",
            Level::Problem,
            "It is not running, so the shortcut does nothing.".into(),
            if windows {
                "Open \"Zenith recorder\" from the Start Menu, or sign out and in again."
            } else {
                "Start it with `systemctl --user enable --now zenithd`."
            },
        ),
    });

    // On Windows the daemon holds the save hotkey itself, and says in its
    // status when another program has taken the combination.
    if windows
        && let Some((state, Some(detail))) = &facts.daemon
        && !matches!(state.as_str(), "failed" | "restarting")
    {
        out.push(check("Save hotkey", Level::Warning, format!("{detail}."), "Pick another key combination in Settings."));
    }

    // Windows plays the sound itself; only Linux needs a player for it.
    if !windows && cfg.feedback.sound && !facts.sound_player {
        out.push(check(
            "Save sound",
            Level::Warning,
            "No sound player (pw-play or paplay) was found, so saving a clip is silent.".into(),
            "Install pipewire or pulseaudio utilities, or turn the sound off in Settings.",
        ));
    }
    out
}

/// The worst of the checks.
pub fn worst(checks: &[Check]) -> Level {
    if checks.iter().any(|c| c.level == Level::Problem) {
        Level::Problem
    } else if checks.iter().any(|c| c.level == Level::Warning) {
        Level::Warning
    } else {
        Level::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Codec;

    fn healthy() -> Facts {
        Facts {
            recorder_installed: true,
            codecs: vec!["h264".into(), "hevc".into(), "h264_vulkan".into()],
            monitors: vec![Output { name: "DP-1".into(), size: (2560, 1440) }],
            app_audio: true,
            ffmpeg: true,
            ffprobe: true,
            x264: true,
            sound_player: true,
            folder_writable: true,
            daemon: Some(("idle".into(), None)),
        }
    }

    /// The checks in Linux's words, which most of these tests are about.
    fn assess_all(facts: &Facts, cfg: &Config) -> Vec<Check> {
        assess_on(facts, cfg, Platform::Linux)
    }

    fn level(checks: &[Check], name: &str) -> Level {
        checks.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no {name} check")).level
    }

    #[test]
    fn a_healthy_machine_passes_everything() {
        let checks = assess_all(&healthy(), &Config::default());
        assert_eq!(worst(&checks), Level::Ok, "{checks:#?}");
        assert!(checks.iter().all(|c| c.fix.is_empty()));
        assert!(checks.iter().any(|c| c.detail == "Recording DP-1 at 2560\u{00D7}1440."));
        assert!(checks.iter().any(|c| c.detail == "Your GPU records H.264, HEVC."));
    }

    #[test]
    fn a_missing_recorder_is_the_one_thing_said_about_it() {
        let facts = Facts { recorder_installed: false, codecs: vec![], monitors: vec![], ..healthy() };
        let checks = assess_all(&facts, &Config::default());
        assert_eq!(level(&checks, "Recorder"), Level::Problem);
        assert!(!checks.iter().any(|c| c.name == "Video encoding" || c.name == "Screen capture"));
        assert_eq!(worst(&checks), Level::Problem);
    }

    #[test]
    fn a_format_the_gpu_lacks_and_a_monitor_that_is_gone_are_problems() {
        let mut cfg = Config::default();
        cfg.capture.codec = Codec::Av1;
        cfg.capture.source = "HDMI-A-1".into();
        let checks = assess_all(&healthy(), &cfg);
        let encoding = checks.iter().find(|c| c.name == "Video encoding").unwrap();
        assert_eq!(encoding.level, Level::Problem);
        assert!(encoding.detail.contains("AV1") && encoding.fix.contains("Settings"), "{encoding:?}");
        assert_eq!(level(&checks, "Screen capture"), Level::Problem);
    }

    #[test]
    fn missing_extras_are_warnings_and_only_when_wanted() {
        let facts = Facts { app_audio: false, sound_player: false, ..healthy() };
        let checks = assess_all(&facts, &Config::default());
        assert_eq!(worst(&checks), Level::Warning);
        assert_eq!(level(&checks, "Sound tracks"), Level::Warning);
        assert_eq!(level(&checks, "Save sound"), Level::Warning);
        let mut cfg = Config::default();
        cfg.audio.chat = false;
        cfg.feedback.sound = false;
        assert_eq!(worst(&assess_all(&facts, &cfg)), Level::Ok);
    }

    #[test]
    fn a_stopped_or_failed_daemon_and_missing_ffmpeg_are_problems() {
        let facts = Facts { daemon: None, ffmpeg: false, ..healthy() };
        let checks = assess_all(&facts, &Config::default());
        assert_eq!(level(&checks, "Background recorder"), Level::Problem);
        assert_eq!(level(&checks, "Editing and export"), Level::Problem);
        let failed = Facts { daemon: Some(("failed".into(), Some("no capture device".into()))), x264: false, folder_writable: false, ..healthy() };
        let checks = assess_all(&failed, &Config::default());
        assert!(checks.iter().any(|c| c.detail.contains("no capture device")));
        assert_eq!(level(&checks, "Editing and export"), Level::Problem);
        assert_eq!(level(&checks, "Clip folder"), Level::Problem);
    }

    fn on_windows(facts: &Facts, cfg: &Config) -> Vec<Check> {
        assess_on(facts, cfg, Platform::Windows)
    }

    fn fix<'a>(checks: &'a [Check], name: &str) -> &'a str {
        &checks.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no {name} check")).fix
    }

    #[test]
    fn a_healthy_windows_machine_passes_with_its_own_recorder() {
        let checks = on_windows(&healthy(), &Config::default());
        assert_eq!(worst(&checks), Level::Ok, "{checks:#?}");
        let names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        assert_eq!(names, ["Recorder", "Video encoding", "Screen capture", "Sound tracks", "Editing and export", "Clip folder", "Background recorder"]);
        assert!(checks.iter().any(|c| c.detail == "zenith-rec is installed."));
    }

    #[test]
    fn windows_fixes_are_about_zenith_not_about_linux_packages() {
        let broken = Facts {
            recorder_installed: false,
            ffmpeg: false,
            sound_player: false,
            daemon: None,
            ..healthy()
        };
        let checks = on_windows(&broken, &Config::default());
        assert_eq!(checks.iter().find(|c| c.name == "Recorder").unwrap().detail, "zenith-rec was not found.");
        assert!(fix(&checks, "Recorder").contains("Reinstall Zenith"));
        assert!(fix(&checks, "Editing and export").contains("Reinstall Zenith"));
        assert!(fix(&checks, "Background recorder").contains("\"Zenith recorder\" from the Start Menu"));
        assert!(fix(&checks, "Background recorder").contains("sign out and in"));
        assert!(!checks.iter().any(|c| c.name == "Save sound"), "Windows plays the sound itself");

        let partly = Facts { codecs: vec![], monitors: vec![], app_audio: false, x264: false, ..healthy() };
        let failed = Facts { daemon: Some(("failed".into(), Some("no capture device".into()))), ..partly };
        let checks = on_windows(&failed, &Config::default());
        assert_eq!(fix(&checks, "Video encoding"), "Update your graphics driver.");
        assert!(fix(&checks, "Screen capture").contains("`zenith-rec --list-capture-options`"));
        assert!(fix(&checks, "Editing and export").contains("Reinstall Zenith"));
        assert!(fix(&checks, "Background recorder").ends_with("in Zenith."));
        assert!(!checks.iter().any(|c| c.name == "Save hotkey"), "a failed recorder's reason is not the hotkey's");
        for check in &checks {
            let text = format!("{} {}", check.detail, check.fix);
            for linux in ["gpu-screen-recorder", "PipeWire", "systemctl", "pw-play", "paplay", "package", "distribution", "tray"] {
                assert!(!text.contains(linux), "{linux} in {check:?}");
            }
        }
    }

    #[test]
    fn a_hotkey_that_is_taken_is_a_warning_on_windows_only() {
        let taken = Facts { daemon: Some(("capturing".into(), Some("The save hotkey Alt+F9 is in use by another program".into()))), ..healthy() };
        let checks = on_windows(&taken, &Config::default());
        assert_eq!(level(&checks, "Save hotkey"), Level::Warning);
        assert_eq!(level(&checks, "Background recorder"), Level::Ok);
        assert!(fix(&checks, "Save hotkey").contains("Settings"));
        assert!(!assess_all(&taken, &Config::default()).iter().any(|c| c.name == "Save hotkey"));
    }

    #[test]
    fn linux_keeps_its_words() {
        let facts = Facts { recorder_installed: false, daemon: None, ffmpeg: false, ..healthy() };
        let checks = assess_all(&facts, &Config::default());
        assert_eq!(fix(&checks, "Recorder"), "Install the gpu-screen-recorder package from your distribution.");
        assert_eq!(fix(&checks, "Background recorder"), "Start it with `systemctl --user enable --now zenithd`.");
        assert_eq!(fix(&checks, "Editing and export"), "Install the ffmpeg package.");
    }
}
