//! The command line, which is `gpu-screen-recorder`'s: the daemon builds one
//! line for both recorders, so every flag it sends must be understood here
//! and every flag it might send one day must at least not stop the recorder.

use std::path::PathBuf;

pub const USAGE: &str = "usage: zenith-rec -w <monitor|screen> -f <fps> [-s <W>x<H>] -k <h264|hevc|av1> -q <kbps> \
-r <seconds> [-cursor yes|no] [-a <source>]... [-game-dir <folder>]... [-game-program <name>]... [-wait-for-game yes|no] -o <directory>
       zenith-rec --list-capture-options | --list-audio-devices | --info | --diagnose
While recording, `save` on stdin writes a clip and prints its path; `stop` or the end of stdin ends it.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub const ALL: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
        }
    }

    /// The codec a `-k` value asks for. The HDR and 10-bit names the Linux
    /// recorder knows are recorded as their plain codec; anything else is
    /// H.264, which every machine can make.
    fn from_arg(value: &str) -> Codec {
        match value {
            v if v.starts_with("hevc") => Self::Hevc,
            v if v.starts_with("av1") => Self::Av1,
            _ => Self::H264,
        }
    }
}

/// What to record and where the clips go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// A monitor's name, or `screen` for the largest.
    pub window: String,
    pub fps: u32,
    /// The size of the picture when it is not the monitor's own.
    pub size: Option<(u32, u32)>,
    pub codec: Codec,
    pub bitrate_kbps: u32,
    /// How many seconds a clip reaches back.
    pub buffer_secs: u32,
    pub cursor: bool,
    /// One sound source a track, in the order the tracks are written.
    pub audio: Vec<String>,
    /// How much later (below zero: earlier) the sound is put against the
    /// picture, in milliseconds.
    pub audio_offset_ms: i64,
    /// Folders games are installed into: a window of a program in a
    /// folder of one of these is a game's, and recorded in place of the
    /// monitor for as long as it is there.
    pub game_dirs: Vec<PathBuf>,
    /// Programs that are games wherever they are, by their file's name.
    pub game_programs: Vec<String>,
    /// Nothing is recorded until a game's window is on screen: the
    /// recorder was started because a game was, and what is on the
    /// monitor while the game starts up is not the game.
    pub wait_for_game: bool,
    pub out_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Record(Options),
    ListCaptureOptions,
    ListAudioDevices,
    Info,
    Diagnose,
    Help,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
    #[error("{0} needs a value")]
    MissingValue(String),
    #[error("{flag} {value}: {why}")]
    BadValue { flag: &'static str, value: String, why: &'static str },
    #[error("-o <directory> is needed: there is nowhere to save clips")]
    NoOutput,
    #[error("unexpected argument {0}")]
    Stray(String),
}

fn number(flag: &'static str, value: &str, range: std::ops::RangeInclusive<u32>) -> Result<u32, ArgError> {
    let bad = |why| ArgError::BadValue { flag, value: value.to_string(), why };
    let n: u32 = value.parse().map_err(|_| bad("is not a whole number"))?;
    if range.contains(&n) { Ok(n) } else { Err(bad("is out of range")) }
}

fn size(value: &str) -> Result<(u32, u32), ArgError> {
    let bad = || ArgError::BadValue { flag: "-s", value: value.to_string(), why: "is not a size like 1920x1080" };
    let (w, h) = value.split_once('x').ok_or_else(bad)?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    if (16..=16384).contains(&w) && (16..=16384).contains(&h) { Ok((w, h)) } else { Err(bad()) }
}

/// Read a command line (without the program's name).
pub fn parse(args: &[String]) -> Result<Command, ArgError> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => return Ok(Command::Help),
        Some("--list-capture-options") => return Ok(Command::ListCaptureOptions),
        Some("--list-audio-devices") => return Ok(Command::ListAudioDevices),
        Some("--info") => return Ok(Command::Info),
        Some("--diagnose") => return Ok(Command::Diagnose),
        _ => {}
    }
    let mut o = Options {
        window: "screen".into(),
        fps: 60,
        size: None,
        codec: Codec::H264,
        bitrate_kbps: 8000,
        buffer_secs: 30,
        cursor: true,
        audio: Vec::new(),
        audio_offset_ms: 0,
        game_dirs: Vec::new(),
        game_programs: Vec::new(),
        wait_for_game: false,
        out_dir: PathBuf::new(),
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        if !flag.starts_with('-') {
            return Err(ArgError::Stray(flag.clone()));
        }
        let value = rest.next().ok_or_else(|| ArgError::MissingValue(flag.clone()))?;
        match flag.as_str() {
            "-w" => o.window = value.clone(),
            "-f" => o.fps = number("-f", value, 1..=240)?,
            "-s" => o.size = Some(size(value)?),
            "-k" => o.codec = Codec::from_arg(value),
            "-q" => o.bitrate_kbps = number("-q", value, 100..=500_000)?,
            "-r" => o.buffer_secs = number("-r", value, 2..=86_400)?,
            "-cursor" => o.cursor = value != "no",
            "-a" => o.audio.push(value.clone()),
            "-audio-offset" => {
                let bad = || ArgError::BadValue { flag: "-audio-offset", value: value.clone(), why: "is not a number of milliseconds within five seconds" };
                o.audio_offset_ms = value.trim().parse::<i64>().ok().filter(|ms| (-5000..=5000).contains(ms)).ok_or_else(bad)?;
            }
            "-game-dir" => o.game_dirs.push(PathBuf::from(value)),
            "-game-program" => o.game_programs.push(value.clone()),
            "-wait-for-game" => o.wait_for_game = value == "yes",
            "-o" => o.out_dir = PathBuf::from(value),
            // What this recorder does in one way only (`-c mp4`, `-bm cbr`,
            // `-keyint 1`, `-ac aac`), or does not do at all.
            _ => {}
        }
    }
    if o.out_dir.as_os_str().is_empty() {
        return Err(ArgError::NoOutput);
    }
    Ok(Command::Record(o))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    fn record(line: &str) -> Options {
        match parse(&words(line)) {
            Ok(Command::Record(o)) => o,
            other => panic!("{line}: {other:?}"),
        }
    }

    #[test]
    fn the_daemons_line_is_understood() {
        let o = record(
            "-w DISPLAY2 -c mp4 -f 60 -s 1920x804 -k hevc -bm cbr -q 22118 -r 45 -replay-storage ram -keyint 1 -ac aac \
             -cursor no -a app-inverse:Discord|app-inverse:Mumble -a app:Discord|app:Mumble -a default_input -v no -o /clips",
        );
        assert_eq!(o.window, "DISPLAY2");
        assert_eq!((o.fps, o.size, o.codec), (60, Some((1920, 804)), Codec::Hevc));
        assert_eq!((o.bitrate_kbps, o.buffer_secs, o.cursor), (22118, 45, false));
        assert_eq!(o.audio, ["app-inverse:Discord|app-inverse:Mumble", "app:Discord|app:Mumble", "default_input"]);
        assert_eq!(o.out_dir, PathBuf::from("/clips"));
    }

    #[test]
    fn what_is_left_out_has_a_default() {
        let o = record("-o /clips");
        assert_eq!((o.window.as_str(), o.fps, o.size, o.codec), ("screen", 60, None, Codec::H264));
        assert!(o.cursor);
        assert!(o.audio.is_empty());
    }

    #[test]
    fn the_sound_can_be_moved_against_the_picture() {
        assert_eq!(record("-w screen -o /clips").audio_offset_ms, 0);
        assert_eq!(record("-audio-offset -120 -o /clips").audio_offset_ms, -120);
        assert!(matches!(parse(&words("-audio-offset soon -o /clips")), Err(ArgError::BadValue { flag: "-audio-offset", .. })));
    }

    #[test]
    fn what_makes_a_program_a_game_is_passed_on() {
        let o = record("-o /clips");
        assert!(o.game_dirs.is_empty() && o.game_programs.is_empty() && !o.wait_for_game);
        assert!(record("-wait-for-game yes -o /c").wait_for_game);
        let args: Vec<String> =
            ["-game-dir", r"C:\Program Files (x86)\Steam\steamapps\common", "-game-dir", r"D:\Games", "-game-program", "star.exe", "-o", "/c"].map(String::from).into();
        let Ok(Command::Record(o)) = parse(&args) else { panic!() };
        assert_eq!(o.game_dirs, [PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common"), PathBuf::from(r"D:\Games")]);
        assert_eq!(o.game_programs, ["star.exe"]);
    }

    #[test]
    fn a_flag_it_does_not_know_is_passed_over_with_its_value() {
        let o = record("-restore-portal-session yes -fm cfr -w screen -o /clips -encoder gpu");
        assert_eq!(o.window, "screen");
        assert_eq!(o.out_dir, PathBuf::from("/clips"));
    }

    #[test]
    fn a_source_name_may_hold_spaces() {
        let args = vec!["-a".to_string(), "app:Team Speak".to_string(), "-o".to_string(), "C:\\My Clips".to_string()];
        let Ok(Command::Record(o)) = parse(&args) else { panic!() };
        assert_eq!(o.audio, ["app:Team Speak"]);
        assert_eq!(o.out_dir, PathBuf::from("C:\\My Clips"));
    }

    #[test]
    fn the_hdr_names_are_their_plain_codec() {
        assert_eq!(record("-k hevc_hdr -o /c").codec, Codec::Hevc);
        assert_eq!(record("-k av1_10bit -o /c").codec, Codec::Av1);
        assert_eq!(record("-k auto -o /c").codec, Codec::H264);
    }

    #[test]
    fn the_questions_are_told_apart() {
        assert_eq!(parse(&words("--list-capture-options")), Ok(Command::ListCaptureOptions));
        assert_eq!(parse(&words("--list-audio-devices")), Ok(Command::ListAudioDevices));
        assert_eq!(parse(&words("--info")), Ok(Command::Info));
        assert_eq!(parse(&words("--diagnose")), Ok(Command::Diagnose));
        assert_eq!(parse(&[]), Ok(Command::Help));
    }

    #[test]
    fn what_cannot_be_recorded_is_refused() {
        assert_eq!(parse(&words("-w screen")), Err(ArgError::NoOutput));
        assert_eq!(parse(&words("-o /c -f")), Err(ArgError::MissingValue("-f".into())));
        assert!(matches!(parse(&words("-o /c -f fast")), Err(ArgError::BadValue { flag: "-f", .. })));
        assert!(matches!(parse(&words("-o /c -f 0")), Err(ArgError::BadValue { flag: "-f", .. })));
        assert!(matches!(parse(&words("-o /c -s 1920")), Err(ArgError::BadValue { flag: "-s", .. })));
        assert!(matches!(parse(&words("-o /c -r 0")), Err(ArgError::BadValue { flag: "-r", .. })));
        assert_eq!(parse(&words("-o /c clip.mp4")), Err(ArgError::Stray("clip.mp4".into())));
    }
}
