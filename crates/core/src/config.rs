//! The settings file. Every field has a default, so an empty or partial file
//! is valid, and values out of range are clamped on load rather than refused.
//! A value that cannot be read at all (a word where a number goes) makes
//! `load` refuse the file; `load_lenient` leaves that one value at its
//! default and keeps the rest.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MIN_BUFFER_SECS: u32 = 15;
pub const MAX_BUFFER_SECS: u32 = 600;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("could not write {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("{0} is not valid: {1}")]
    Parse(PathBuf, String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Config {
    pub capture: Capture,
    pub audio: Audio,
    pub library: Library,
    pub feedback: Feedback,
    pub updates: Updates,
    pub games: Games,
    pub look: Look,
    pub editor: Editor,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Capture {
    /// How many seconds before the shortcut a clip reaches back.
    pub buffer_secs: u32,
    pub resolution: Resolution,
    pub fps: u32,
    pub codec: Codec,
    pub quality: Quality,
    /// `screen` (the largest monitor), a monitor name such as `DP-1`, or `portal`.
    pub source: String,
    pub activation: Activation,
    pub cursor: bool,
    /// The key combination that saves a clip on Windows, where the daemon
    /// holds it itself (`Alt+F9`; see `crate::hotkey`). Linux desktops bind
    /// `zenithd save` to a key in their own settings, so it is not used there.
    pub hotkey: String,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            buffer_secs: 60,
            resolution: Resolution::Native,
            fps: 60,
            codec: Codec::H264,
            quality: Quality::High,
            source: "screen".into(),
            activation: Activation::Game,
            cursor: true,
            hotkey: crate::hotkey::DEFAULT.into(),
        }
    }
}

/// The recording size. `Native` records the monitor as it is; anything else
/// is a box the picture is scaled to fit in, keeping its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Native,
    Size(u32, u32),
}

impl Resolution {
    /// The 16:9 box of a given height: `height(1080)` is 1920x1080.
    pub fn height(h: u32) -> Self {
        Self::Size((h * 16 / 9 + 1) & !1, h)
    }

    /// What is recorded on a monitor of `native` size: the picture scaled
    /// down to fit the box, never up, with even sides (encoders need them).
    pub fn on(self, native: (u32, u32)) -> (u32, u32) {
        let (nw, nh) = native;
        match self {
            Self::Size(w, h) if nw > 0 && nh > 0 && (w < nw || h < nh) => {
                let scale = (w as f64 / nw as f64).min(h as f64 / nh as f64);
                let even = |v: u32| (((v as f64 * scale).round() as u32).max(2)) & !1;
                (even(nw), even(nh))
            }
            _ => native,
        }
    }
}

impl std::fmt::Display for Resolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native => f.write_str("native"),
            Self::Size(w, h) => write!(f, "{w}x{h}"),
        }
    }
}

impl std::str::FromStr for Resolution {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        if s == "native" {
            return Ok(Self::Native);
        }
        if let Some(h) = s.strip_suffix('p').and_then(|h| h.parse::<u32>().ok()) {
            return if (144..=4320).contains(&h) {
                Ok(Self::height(h))
            } else {
                Err(format!("{h}p is out of range"))
            };
        }
        let (w, h) = s.split_once('x').ok_or_else(|| format!("\"{s}\" is not native, 1080p or 1920x1080"))?;
        match (w.parse::<u32>(), h.parse::<u32>()) {
            (Ok(w), Ok(h)) if (256..=7680).contains(&w) && (144..=4320).contains(&h) => Ok(Self::Size(w, h)),
            _ => Err(format!("\"{s}\" is not a usable size")),
        }
    }
}

impl Serialize for Resolution {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Resolution {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
        }
    }

    /// How many bits it needs next to H.264 for the same picture.
    fn efficiency(self) -> f64 {
        match self {
            Self::H264 => 1.0,
            Self::Hevc => 0.7,
            Self::Av1 => 0.6,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

impl Quality {
    /// Bits per pixel per frame for H.264.
    fn bits_per_pixel(self) -> f64 {
        match self {
            Self::Low => 0.04,
            Self::Medium => 0.07,
            Self::High => 0.10,
            Self::Ultra => 0.15,
        }
    }
}

/// When the replay buffer runs.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Activation {
    /// Only while a game is running: nothing is encoded on the desktop.
    Game,
    Always,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Audio {
    /// Everything that plays, except the voice apps when `chat` is on.
    pub game: bool,
    /// The voice apps, on a track of their own.
    pub chat: bool,
    pub microphone: bool,
    /// Application names as PipeWire reports them.
    pub voice_apps: Vec<String>,
    pub output_device: String,
    pub input_device: String,
    /// How much later (or, below zero, earlier) the sound is put against
    /// the picture, in milliseconds, where the two do not line up by
    /// themselves. Only Zenith's own recorder (Windows) needs it.
    pub offset_ms: i32,
}

/// The furthest the sound may be moved against the picture, either way.
pub const MAX_SOUND_OFFSET_MS: i32 = 1000;

impl Default for Audio {
    fn default() -> Self {
        Self {
            game: true,
            chat: true,
            microphone: true,
            voice_apps: ["Discord", "Vesktop", "WEBRTC VoiceEngine", "TeamSpeak", "Mumble"].map(String::from).into(),
            output_device: "default_output".into(),
            input_device: "default_input".into(),
            offset_ms: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Library {
    pub dir: PathBuf,
}

impl Default for Library {
    fn default() -> Self {
        Self { dir: crate::paths::default_clip_dir() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Feedback {
    pub sound: bool,
    pub notification: bool,
}

impl Default for Feedback {
    fn default() -> Self {
        Self { sound: true, notification: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Updates {
    /// Look for a new version now and then. Nothing else is ever fetched.
    pub check: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self { check: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Games {
    /// Games added by hand, for the ones not found by themselves.
    pub added: Vec<AddedGame>,
}

/// A game known by the program it runs.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct AddedGame {
    /// What its clips are filed under.
    pub name: String,
    /// Its program's file name (`Game.exe`), in any case, with or without
    /// `.exe`.
    pub program: String,
}

/// The app's colours, where the user wants their own: `#RRGGBB`, or
/// nothing for Zenith's.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Look {
    /// What says what is: names, what is chosen, the kept part of a clip.
    pub accent: String,
    /// What says what can be done: buttons, switches, sliders.
    pub action: String,
    /// The window's own colour, behind everything.
    pub background: String,
    /// The lines between the parts of the window and around its boxes.
    pub line: String,
}

/// Where the editor starts with a clip that has not been edited yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Editor {
    /// How loud each track starts: 1.0 is as recorded, 0 to 2. A clip keeps
    /// its own volumes from the first change to it.
    pub game_volume: f64,
    pub chat_volume: f64,
    pub microphone_volume: f64,
}

impl Default for Editor {
    fn default() -> Self {
        Self { game_volume: 1.0, chat_volume: 1.0, microphone_volume: 1.0 }
    }
}

impl Editor {
    /// The volume a track of this kind starts at.
    pub fn initial_volume(&self, track: crate::recorder::Track) -> f64 {
        use crate::recorder::Track;
        match track {
            Track::Game => self.game_volume,
            Track::Chat => self.chat_volume,
            Track::Microphone => self.microphone_volume,
        }
    }
}

/// A colour as `#RRGGBB`; nothing for what is not one.
fn colour(text: &str) -> String {
    let text = text.trim();
    match text.strip_prefix('#') {
        Some(hex) if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => format!("#{}", hex.to_ascii_uppercase()),
        _ => String::new(),
    }
}

/// A colour written `#RRGGBB`, as its red, green and blue; nothing for
/// what is not one (also for no colour chosen, which is written empty).
pub fn rgb(text: &str) -> Option<(u8, u8, u8)> {
    let hex = colour(text);
    let part = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    Some((part(1)?, part(3)?, part(5)?))
}

impl Capture {
    /// Whether the recorder would be run the same for both: everything but
    /// the hotkey, which is the daemon's own and no reason to restart it.
    pub fn records_like(&self, other: &Self) -> bool {
        let without_hotkey = |c: &Self| Self { hotkey: String::new(), ..c.clone() };
        without_hotkey(self) == without_hotkey(other)
    }

    /// The constant video bitrate for this quality on a monitor of `native` size.
    pub fn bitrate_kbps(&self, native: (u32, u32)) -> u32 {
        let (w, h) = self.resolution.on(native);
        let bits = w as f64 * h as f64 * self.fps as f64 * self.quality.bits_per_pixel() * self.codec.efficiency();
        ((bits / 1000.0).round() as u32).clamp(1_000, 150_000)
    }

    /// Roughly how much memory the replay buffer holds, in megabytes: the
    /// video at its constant bitrate plus `audio_tracks` at 128 kbit/s.
    pub fn buffer_megabytes(&self, native: (u32, u32), audio_tracks: u32) -> u32 {
        let kbps = self.bitrate_kbps(native) as u64 + 128 * audio_tracks as u64;
        (kbps * self.buffer_secs as u64 / 8 / 1000) as u32
    }
}

impl Config {
    /// Bring every value into its range. Returns whether anything changed.
    pub fn clamp(&mut self) -> bool {
        let before = self.clone();
        let c = &mut self.capture;
        c.buffer_secs = c.buffer_secs.clamp(MIN_BUFFER_SECS, MAX_BUFFER_SECS);
        c.fps = c.fps.clamp(10, 240);
        if c.source.trim().is_empty() {
            c.source = "screen".into();
        }
        self.audio.voice_apps.retain(|a| !a.trim().is_empty() && !a.contains('|'));
        self.audio.offset_ms = self.audio.offset_ms.clamp(-MAX_SOUND_OFFSET_MS, MAX_SOUND_OFFSET_MS);
        if self.library.dir.as_os_str().is_empty() {
            self.library.dir = crate::paths::default_clip_dir();
        }
        // An added game is a program's file name and a name to file under;
        // each program once.
        use crate::games::programs::{bare, file_name};
        let mut known = Vec::new();
        for game in &mut self.games.added {
            game.program = file_name(&game.program).to_string();
            game.name = game.name.trim().to_string();
            if game.name.is_empty() {
                game.name = game.program.rsplit_once('.').map_or(game.program.as_str(), |(stem, _)| stem).to_string();
            }
        }
        self.games.added.retain(|game| {
            let program = bare(&game.program);
            let keep = !program.is_empty() && !game.name.is_empty() && !known.contains(&program);
            known.push(program);
            keep
        });
        self.look.accent = colour(&self.look.accent);
        self.look.action = colour(&self.look.action);
        self.look.background = colour(&self.look.background);
        self.look.line = colour(&self.look.line);
        for volume in [&mut self.editor.game_volume, &mut self.editor.chat_volume, &mut self.editor.microphone_volume] {
            *volume = if volume.is_finite() { volume.clamp(0.0, 2.0) } else { 1.0 };
        }
        *self != before
    }

    /// The settings in `path`; the defaults when there is no file yet.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::Read(path.into(), e)),
        };
        let mut cfg: Self = toml::from_str(&text).map_err(|e| ConfigError::Parse(path.into(), e.to_string()))?;
        cfg.clamp();
        Ok(cfg)
    }

    /// The settings in `path`, with every value that can be read: one that
    /// cannot is left at its default and named in the list that comes back
    /// (`capture.fps`). Only a file that is not TOML at all is refused.
    pub fn load_lenient(path: &Path) -> Result<(Self, Vec<String>), ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Self::default(), Vec::new())),
            Err(e) => return Err(ConfigError::Read(path.into(), e)),
        };
        let (mut cfg, ignored) = Self::read_lenient(&text).map_err(|e| ConfigError::Parse(path.into(), e))?;
        cfg.clamp();
        Ok((cfg, ignored))
    }

    /// The settings to work with, whatever state the file is in: every
    /// value that can be read, and the defaults for the rest. What the
    /// window uses, so one value it cannot read (a typing mistake, or one
    /// written by a newer Zenith) does not put every setting back to its
    /// default, and then over the file at the next change.
    pub fn load_usable(path: &Path) -> Self {
        Self::load_lenient(path).map(|(cfg, _)| cfg).unwrap_or_default()
    }

    fn read_lenient(text: &str) -> Result<(Self, Vec<String>), String> {
        let read = |table: &toml::Table| toml::Value::Table(table.clone()).try_into::<Self>();
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        if let Ok(cfg) = read(&table) {
            return Ok((cfg, Vec::new()));
        }
        // Put the values in one at a time, keeping each that still reads.
        let mut good = toml::Table::new();
        let mut ignored = Vec::new();
        for (section, value) in table {
            let toml::Value::Table(values) = value else {
                ignored.push(section);
                continue;
            };
            for (key, value) in values {
                let with = |value: toml::Value| {
                    let mut trial = good.clone();
                    let slot = trial.entry(section.clone()).or_insert_with(|| toml::Value::Table(Default::default()));
                    if let toml::Value::Table(slot) = slot {
                        slot.insert(key.clone(), value);
                    }
                    trial
                };
                let trial = with(value.clone());
                if read(&trial).is_ok() {
                    good = trial;
                    continue;
                }
                // A list is not lost for one entry in it (an added game
                // with a number for a name): the entries are put in one at
                // a time too. Where no list belongs at all, not even an
                // empty one reads, and the whole value is left out.
                let entries = match value {
                    toml::Value::Array(entries) if read(&with(toml::Value::Array(Vec::new()))).is_ok() => entries,
                    _ => {
                        ignored.push(format!("{section}.{key}"));
                        continue;
                    }
                };
                let mut kept = Vec::new();
                for (at, entry) in entries.into_iter().enumerate() {
                    kept.push(entry);
                    if read(&with(toml::Value::Array(kept.clone()))).is_err() {
                        kept.pop();
                        ignored.push(format!("{section}.{key}[{at}]"));
                    }
                }
                good = with(toml::Value::Array(kept));
            }
        }
        let cfg = read(&good).map_err(|e| e.to_string())?;
        Ok((cfg, ignored))
    }

    /// Write the file whole: to a temporary one beside it, then renamed over.
    /// The temporary file is this process's and this call's own (the window
    /// and the recorder both write the settings), and is on the disk before
    /// it takes the file's place, so a power cut leaves the old settings or
    /// the new ones and never half a file.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        use std::io::Write;
        let err = |e| ConfigError::Write(path.into(), e);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        // A clip folder whose name is not text cannot be written as TOML:
        // that is said, and the file stays as it was.
        let text = toml::to_string_pretty(self).map_err(|e| err(std::io::Error::other(e.to_string())))?;
        // A file with something in it that was not understood is about to
        // lose that: a copy of it is kept beside it, as it was.
        if let Ok(old) = std::fs::read_to_string(path)
            && toml::from_str::<Self>(&old).is_err()
        {
            let _ = std::fs::write(path.with_extension("toml.bak"), old);
        }
        static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("toml.{}-{call}.tmp", std::process::id()));
        let written = std::fs::File::create(&tmp)
            .and_then(|mut file| {
                file.write_all(text.as_bytes())?;
                file.sync_all()
            })
            .and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(err(e));
        }
        // The new name is on the disk too once its folder is. Not every
        // system can do that for a folder, and the file is whole either way.
        #[cfg(unix)]
        if let Some(dir) = path.parent()
            && let Ok(dir) = std::fs::File::open(dir)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_read_as_its_three_parts() {
        assert_eq!(rgb("#3DE0F5"), Some((0x3D, 0xE0, 0xF5)));
        assert_eq!(rgb(" #f1efe4 "), Some((0xF1, 0xEF, 0xE4)));
        assert_eq!(rgb(""), None, "no colour chosen");
        assert_eq!(rgb("#12345"), None);
        assert_eq!(rgb("cyan"), None);
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn added_games_and_colours_are_kept_tidy() {
        let text = r##"
            [look]
            accent = " #6fe08a "
            action = "pink"

            [[games.added]]
            name = " Order of the Sinking Star "
            program = 'C:\Games\Star\star.exe'

            [[games.added]]
            name = "The same again"
            program = "STAR"

            [[games.added]]
            program = "other.exe"

            [[games.added]]
            name = "No program"
        "##;
        let mut cfg: Config = toml::from_str(text).unwrap();
        assert!(cfg.clamp());
        let games: Vec<(&str, &str)> = cfg.games.added.iter().map(|g| (g.name.as_str(), g.program.as_str())).collect();
        assert_eq!(games, [("Order of the Sinking Star", "star.exe"), ("other", "other.exe")]);
        assert_eq!((cfg.look.accent.as_str(), cfg.look.action.as_str()), ("#6FE08A", ""));
        // What was written is read back the same.
        let again: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(again, cfg);
        assert!(!cfg.clone().clamp());
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let cfg: Config = toml::from_str("[capture]\nbuffer_secs = 30\nresolution = \"1080p\"\ncodec = \"hevc\"").unwrap();
        assert_eq!(cfg.capture.buffer_secs, 30);
        assert_eq!(cfg.capture.resolution, Resolution::Size(1920, 1080));
        assert_eq!(cfg.capture.codec, Codec::Hevc);
        assert_eq!(cfg.capture.fps, 60);
    }

    #[test]
    fn it_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut cfg = Config::default();
        cfg.capture.resolution = Resolution::Size(2560, 1440);
        cfg.capture.activation = Activation::Always;
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), cfg);
    }

    #[test]
    fn it_reads_and_writes_as_json_for_the_screens() {
        let mut cfg = Config::default();
        cfg.capture.resolution = Resolution::Size(1920, 1080);
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["capture"]["resolution"], "1920x1080");
        assert_eq!(json["capture"]["activation"], "game");
        assert_eq!(serde_json::from_value::<Config>(json).unwrap(), cfg);
        let partial: Config = serde_json::from_str(r#"{"capture":{"fps":30,"resolution":"native"}}"#).unwrap();
        assert_eq!((partial.capture.fps, partial.capture.resolution), (30, Resolution::Native));
        assert!(serde_json::from_str::<Config>(r#"{"capture":{"codec":"mpeg2"}}"#).is_err());
    }

    #[test]
    fn the_hotkey_has_a_default_and_is_kept_as_written() {
        assert_eq!(Config::default().capture.hotkey, "Alt+F9");
        let cfg: Config = toml::from_str("[capture]\nhotkey = \"ctrl+shift+s\"").unwrap();
        assert_eq!(cfg.capture.hotkey, "ctrl+shift+s");
        // One that cannot be read is still loaded: who uses it falls back.
        let mut odd: Config = toml::from_str("[capture]\nhotkey = \"Alt+Banana\"").unwrap();
        assert!(!odd.clamp());
        assert_eq!(odd.capture.hotkey, "Alt+Banana");
    }

    #[test]
    fn a_new_hotkey_is_no_reason_to_restart_the_recorder() {
        let a = Capture::default();
        let b = Capture { hotkey: "Ctrl+F10".into(), ..a.clone() };
        assert!(a != b && a.records_like(&b));
        assert!(!a.records_like(&Capture { fps: 30, ..a.clone() }));
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let mut cfg: Config = toml::from_str("[capture]\nbuffer_secs = 5\nfps = 1000\nsource = \"\"").unwrap();
        assert!(cfg.clamp());
        assert_eq!((cfg.capture.buffer_secs, cfg.capture.fps), (MIN_BUFFER_SECS, 240));
        assert_eq!(cfg.capture.source, "screen");
    }

    #[test]
    fn a_bad_file_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[capture]\nresolution = \"huge\"").unwrap();
        assert!(matches!(Config::load(&path), Err(ConfigError::Parse(..))));
    }

    #[test]
    fn the_window_keeps_what_it_can_read_and_a_copy_of_what_it_could_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let written = "[capture]\nfps = 30\ncodec = \"from-a-newer-zenith\"\n[library]\ndir = \"/clips\"\n";
        std::fs::write(&path, written).unwrap();
        let cfg = Config::load_usable(&path);
        assert_eq!(cfg.capture.fps, 30);
        assert_eq!(cfg.library.dir, PathBuf::from("/clips"));
        // Saved again, the file is as the window read it, and the one it
        // was made from is still there to look at.
        let bak = dir.path().join("config.toml.bak");
        cfg.save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), written);
        assert_eq!(Config::load(&path).unwrap(), cfg);
        // A file that reads whole is not copied.
        std::fs::remove_file(&bak).unwrap();
        cfg.save(&path).unwrap();
        assert!(!bak.exists());
        // Not TOML at all: the defaults, and the file kept when they are saved.
        std::fs::write(&path, "[capture\nfps = ").unwrap();
        assert_eq!(Config::load_usable(&path), Config::default());
        Config::default().save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "[capture\nfps = ");
    }

    #[test]
    fn one_bad_value_does_not_cost_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[capture]\nbuffer_secs = -5\nfps = 30\ncodec = \"mpeg2\"\nactivation = \"always\"\n[library]\ndir = \"/clips\"\n[feedback]\nsound = \"loud\"\n",
        )
        .unwrap();
        assert!(Config::load(&path).is_err());
        let (cfg, ignored) = Config::load_lenient(&path).unwrap();
        assert_eq!(ignored, ["capture.buffer_secs", "capture.codec", "feedback.sound"]);
        assert_eq!((cfg.capture.fps, cfg.capture.activation), (30, Activation::Always));
        assert_eq!(cfg.library.dir, PathBuf::from("/clips"));
        assert_eq!((cfg.capture.buffer_secs, cfg.capture.codec, cfg.feedback.sound), (60, Codec::H264, true));

        std::fs::write(&path, "[capture]\nfps = 30").unwrap();
        assert_eq!(Config::load_lenient(&path).unwrap().1, Vec::<String>::new());
        std::fs::write(&path, "this is not = = toml").unwrap();
        assert!(matches!(Config::load_lenient(&path), Err(ConfigError::Parse(..))));
        assert_eq!(Config::load_lenient(&dir.path().join("none.toml")).unwrap().0, Config::default());
    }

    #[test]
    fn one_bad_entry_in_a_list_does_not_cost_the_others() {
        let text = r#"
            [audio]
            voice_apps = ["Discord", 7, "Mumble"]

            [capture]
            fps = [30, 60]

            [[games.added]]
            name = "Hades"
            program = "hades.exe"

            [[games.added]]
            name = 5
            program = "five.exe"

            [[games.added]]
            name = "Celeste"
            program = "celeste.exe"
        "#;
        assert!(toml::from_str::<Config>(text).is_err());
        let (cfg, ignored) = Config::read_lenient(text).unwrap();
        assert_eq!(ignored, ["audio.voice_apps[1]", "capture.fps", "games.added[1]"]);
        let games: Vec<&str> = cfg.games.added.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(games, ["Hades", "Celeste"]);
        assert_eq!(cfg.audio.voice_apps, ["Discord", "Mumble"]);
        assert_eq!(cfg.capture.fps, 60, "a list where a number goes is left out whole");
    }

    #[test]
    fn saving_leaves_only_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // The window and the recorder, saving at the same moment.
        std::thread::scope(|all| {
            for fps in [30, 60, 120, 144] {
                let path = &path;
                all.spawn(move || {
                    let mut cfg = Config::default();
                    cfg.capture.fps = fps;
                    for _ in 0..20 {
                        cfg.save(path).unwrap();
                    }
                });
            }
        });
        assert!([30, 60, 120, 144].contains(&Config::load(&path).unwrap().capture.fps), "one of them, whole");
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["config.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_clip_folder_that_cannot_be_written_down_is_an_error_and_keeps_the_file() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::default().save(&path).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let mut cfg = Config::default();
        cfg.library.dir = PathBuf::from(std::ffi::OsStr::from_bytes(b"/clips/\xff\xfe"));
        assert!(matches!(cfg.save(&path), Err(ConfigError::Write(..))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn resolutions_parse_and_fit() {
        assert_eq!("native".parse(), Ok(Resolution::Native));
        assert_eq!("1440p".parse(), Ok(Resolution::Size(2560, 1440)));
        assert_eq!("720p".parse(), Ok(Resolution::Size(1280, 720)));
        assert_eq!("1920X1080".parse(), Ok(Resolution::Size(1920, 1080)));
        assert!("10p".parse::<Resolution>().is_err());
        // An ultrawide in a 1080p box keeps its shape; nothing is scaled up.
        assert_eq!(Resolution::height(1080).on((3440, 1440)), (1920, 804));
        assert_eq!(Resolution::height(1440).on((1920, 1080)), (1920, 1080));
        assert_eq!(Resolution::Native.on((2560, 1440)), (2560, 1440));
    }

    #[test]
    fn bitrate_follows_size_rate_and_codec() {
        let mut c = Capture::default();
        assert_eq!(c.bitrate_kbps((2560, 1440)), 22_118);
        c.codec = Codec::Hevc;
        assert_eq!(c.bitrate_kbps((2560, 1440)), 15_483);
        c.resolution = Resolution::height(1080);
        c.fps = 30;
        assert!(c.bitrate_kbps((2560, 1440)) < 5_000);
        // 60 s at 22 Mbit/s with three audio tracks.
        let d = Capture::default();
        assert_eq!(d.buffer_megabytes((2560, 1440), 3), 168);
    }
}
