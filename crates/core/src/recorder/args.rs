//! gpu-screen-recorder's command line for a replay buffer.

use crate::config::{Config, Resolution};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What an audio track of a clip holds, in the order they are recorded.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    Game,
    Chat,
    Microphone,
}

impl Track {
    pub fn label(self) -> &'static str {
        match self {
            Self::Game => "Game",
            Self::Chat => "Voice chat",
            Self::Microphone => "Microphone",
        }
    }
}

/// The audio tracks these settings record, each with the recorder's `-a` value.
pub fn tracks(cfg: &Config) -> Vec<(Track, String)> {
    let a = &cfg.audio;
    let split_chat = a.chat && !a.voice_apps.is_empty();
    let mut out = Vec::new();
    if a.game {
        let source = if split_chat {
            // Every application except the voice ones.
            a.voice_apps.iter().map(|v| format!("app-inverse:{v}")).collect::<Vec<_>>().join("|")
        } else {
            a.output_device.clone()
        };
        out.push((Track::Game, source));
    }
    if split_chat {
        out.push((Track::Chat, a.voice_apps.iter().map(|v| format!("app:{v}")).collect::<Vec<_>>().join("|")));
    }
    if a.microphone {
        out.push((Track::Microphone, a.input_device.clone()));
    }
    out
}

/// The arguments that keep a replay buffer for these settings on a monitor of
/// `native` size, saving clips into `out_dir`.
pub fn command_line(cfg: &Config, native: (u32, u32), out_dir: &Path) -> Vec<String> {
    let c = &cfg.capture;
    let mut v: Vec<String> = Vec::new();
    let mut arg = |k: &str, val: String| {
        v.push(k.into());
        v.push(val);
    };
    arg("-w", c.source.clone());
    arg("-c", "mp4".into());
    arg("-f", c.fps.to_string());
    if let Resolution::Size(..) = c.resolution {
        let (w, h) = c.resolution.on(native);
        if (w, h) != native {
            arg("-s", format!("{w}x{h}"));
        }
    }
    arg("-k", c.codec.as_str().into());
    // A constant bitrate, so the buffer's memory is the same whatever is on screen.
    arg("-bm", "cbr".into());
    arg("-q", c.bitrate_kbps(native).to_string());
    arg("-r", c.buffer_secs.to_string());
    arg("-replay-storage", "ram".into());
    // A keyframe a second: cuts land close to where they are asked for.
    arg("-keyint", "1".into());
    arg("-ac", "aac".into());
    arg("-cursor", if c.cursor { "yes" } else { "no" }.into());
    if c.source == "portal" {
        arg("-restore-portal-session", "yes".into());
    }
    for (_, source) in tracks(cfg) {
        arg("-a", source);
    }
    arg("-v", "no".into());
    // Zenith's own recorder alone takes these; gpu-screen-recorder would
    // refuse a flag it does not know.
    if cfg!(windows) {
        for pair in for_zenith_rec(cfg, &game_folders()).chunks(2) {
            arg(&pair[0], pair[1].clone());
        }
    }
    arg("-o", out_dir.to_string_lossy().into_owned());
    v
}

/// The folders games are in: the ones Steam installs into, and the folder
/// of each game another launcher has installed.
pub fn game_folders() -> Vec<PathBuf> {
    let mut folders = steam_folders();
    for game in crate::games::launchers::installed() {
        if !folders.contains(&game.dir) {
            folders.push(game.dir);
        }
    }
    folders
}

/// The folders Steam installs games into: `steamapps\common` of its own
/// library and of each one added on another drive.
fn steam_folders() -> Vec<PathBuf> {
    use crate::games::steam;
    let mut folders = Vec::new();
    for root in steam::roots(&crate::paths::home()) {
        for library in steam::library_dirs(&root) {
            let folder = library.join("common");
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
    }
    folders
}

/// What only Zenith's own recorder has a flag for: how far to move the
/// sound, and what makes a program a game. It records a game's window in
/// place of the monitor while there is one, and knows a game the way the
/// daemon does: by its program being in one of `game_folders`, or being
/// one the user added.
pub fn for_zenith_rec(cfg: &Config, game_folders: &[PathBuf]) -> Vec<String> {
    let mut v = Vec::new();
    if cfg.audio.offset_ms != 0 {
        v.extend(["-audio-offset".to_string(), cfg.audio.offset_ms.to_string()]);
    }
    for folder in game_folders {
        v.extend(["-game-dir".to_string(), folder.to_string_lossy().into_owned()]);
    }
    for game in &cfg.games.added {
        v.extend(["-game-program".to_string(), game.program.clone()]);
    }
    // Started because a game was: what is on the monitor while the game
    // starts up is not the game, so nothing is recorded until its window
    // is there.
    if cfg.capture.activation == crate::config::Activation::Game {
        v.extend(["-wait-for-game".to_string(), "yes".to_string()]);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Codec, Config};

    fn value<'a>(args: &'a [String], key: &str) -> Vec<&'a str> {
        args.windows(2).filter(|w| w[0] == key).map(|w| w[1].as_str()).collect()
    }

    #[test]
    fn zeniths_own_recorder_is_told_how_far_to_move_the_sound() {
        let mut cfg = Config::default();
        cfg.capture.activation = crate::config::Activation::Always;
        assert!(for_zenith_rec(&cfg, &[]).is_empty(), "nothing is said when there is nothing to move");
        cfg.audio.offset_ms = -120;
        assert_eq!(for_zenith_rec(&cfg, &[]), ["-audio-offset", "-120"]);
    }

    #[test]
    fn zeniths_own_recorder_is_told_what_makes_a_program_a_game() {
        let mut cfg = Config::default();
        cfg.capture.activation = crate::config::Activation::Always;
        cfg.games.added.push(crate::config::AddedGame { name: "Star".into(), program: "star.exe".into() });
        let folders = [PathBuf::from("C:/Steam/steamapps/common"), PathBuf::from("D:/Library/steamapps/common")];
        assert_eq!(
            for_zenith_rec(&cfg, &folders),
            ["-game-dir", "C:/Steam/steamapps/common", "-game-dir", "D:/Library/steamapps/common", "-game-program", "star.exe"]
        );
    }

    #[test]
    fn started_for_a_game_it_waits_for_the_games_window() {
        let mut cfg = Config::default();
        cfg.capture.activation = crate::config::Activation::Game;
        assert_eq!(for_zenith_rec(&cfg, &[]), ["-wait-for-game", "yes"]);
        cfg.capture.activation = crate::config::Activation::Always;
        assert!(for_zenith_rec(&cfg, &[]).is_empty(), "recording all the time, the monitor is what there is to record");
    }

    #[test]
    fn the_folders_games_are_installed_into_are_each_named_once() {
        let folders = steam_folders();
        assert!(folders.iter().all(|folder| folder.ends_with("steamapps/common")), "{folders:?}");
        let mut once = folders.clone();
        once.dedup();
        assert_eq!(once, folders);
    }

    #[test]
    fn the_defaults_make_a_ram_replay_buffer() {
        let args = command_line(&Config::default(), (2560, 1440), Path::new("/tmp/out"));
        assert_eq!(value(&args, "-w"), ["screen"]);
        assert_eq!(value(&args, "-r"), ["60"]);
        assert_eq!(value(&args, "-replay-storage"), ["ram"]);
        assert_eq!(value(&args, "-bm"), ["cbr"]);
        assert_eq!(value(&args, "-q"), ["22118"]);
        assert_eq!(value(&args, "-f"), ["60"]);
        assert_eq!(value(&args, "-k"), ["h264"]);
        assert_eq!(value(&args, "-o"), ["/tmp/out"]);
        assert!(value(&args, "-s").is_empty(), "native needs no size");
        assert!(value(&args, "-restore-portal-session").is_empty());
    }

    #[test]
    fn three_tracks_split_the_voice_apps_out() {
        let mut cfg = Config::default();
        cfg.audio.voice_apps = vec!["Discord".into(), "Mumble".into()];
        let args = command_line(&cfg, (1920, 1080), Path::new("/o"));
        assert_eq!(
            value(&args, "-a"),
            ["app-inverse:Discord|app-inverse:Mumble", "app:Discord|app:Mumble", "default_input"]
        );
        assert_eq!(tracks(&cfg).iter().map(|t| t.0).collect::<Vec<_>>(), [Track::Game, Track::Chat, Track::Microphone]);
    }

    #[test]
    fn without_a_chat_track_the_game_track_is_the_whole_output() {
        let mut cfg = Config::default();
        cfg.audio.chat = false;
        cfg.audio.microphone = false;
        assert_eq!(tracks(&cfg), [(Track::Game, "default_output".to_string())]);
        cfg.audio.game = false;
        assert!(tracks(&cfg).is_empty());
    }

    #[test]
    fn a_smaller_size_and_another_codec_are_passed_on() {
        let mut cfg = Config::default();
        cfg.capture.resolution = "1080p".parse().unwrap();
        cfg.capture.codec = Codec::Av1;
        cfg.capture.source = "portal".into();
        cfg.capture.cursor = false;
        let args = command_line(&cfg, (3440, 1440), Path::new("/o"));
        assert_eq!(value(&args, "-s"), ["1920x804"]);
        assert_eq!(value(&args, "-k"), ["av1"]);
        assert_eq!(value(&args, "-cursor"), ["no"]);
        assert_eq!(value(&args, "-restore-portal-session"), ["yes"]);
        // A box bigger than the monitor changes nothing.
        cfg.capture.resolution = "2160p".parse().unwrap();
        assert!(value(&command_line(&cfg, (1920, 1080), Path::new("/o")), "-s").is_empty());
    }
}
