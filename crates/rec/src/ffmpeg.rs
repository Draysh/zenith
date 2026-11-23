//! Every ffmpeg command line this program runs, built as plain lists of
//! words so they can be read in a test, and the few questions asked of
//! ffmpeg itself: where it is, what it was built with, whether a way of
//! capturing and encoding works on this machine.
//!
//! The capture filters are used as ffmpeg's filter manual gives them:
//! `ddagrab` (a monitor) and `gfxcapture` (a monitor, or one window) both
//! hand out Direct3D 11 frames, which the
//! NVIDIA and AMD encoders take as they are (the manual's own examples) and
//! everything else needs `hwdownload,format=bgra` in front.

use crate::args::Codec;
use crate::ring::Container;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// ffmpeg's own file name here.
const PROGRAM: &str = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };

/// The ffmpeg to run: the one shipped beside this program, or else
/// whichever is on the PATH.
pub fn locate() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join(PROGRAM)));
    beside.filter(|path| path.is_file()).unwrap_or_else(|| PathBuf::from(PROGRAM))
}

/// A command for `program` that never opens a window: started from a
/// daemon with no console, a console program gets a new one otherwise,
/// flashing over the game for each ffmpeg we start.
pub fn command(program: &Path) -> Command {
    command_with(program, 0)
}

/// The same for the ffmpeg that records: one that also runs before
/// ordinary programs. A game takes every processor it can get, and an
/// ffmpeg that waits its turn behind it drops frames and reads the sound
/// late.
pub fn recording_command(program: &Path) -> Command {
    const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
    command_with(program, ABOVE_NORMAL_PRIORITY_CLASS)
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn command_with(program: &Path, more_flags: u32) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW | more_flags);
    }
    command
}

/// What this ffmpeg was built with.
#[derive(Clone, Debug, Default)]
pub struct Abilities {
    pub version: String,
    pub filters: HashSet<String>,
    pub encoders: HashSet<String>,
}

/// The filter names in `ffmpeg -filters`: lines like ` T.. ddagrab  |->V  Grab...`.
pub fn parse_filters(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (_flags, name, shape) = (words.next()?, words.next()?, words.next()?);
            shape.contains("->").then(|| name.to_string())
        })
        .collect()
}

/// The encoder names in `ffmpeg -encoders`: lines like ` V....D h264_nvenc  NVIDIA...`.
pub fn parse_encoders(text: &str) -> HashSet<String> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (flags, name) = (words.next()?, words.next()?);
            (flags.len() == 6 && flags.starts_with(['V', 'A', 'S']) && name != "=").then(|| name.to_string())
        })
        .collect()
}

fn ask(ffmpeg: &Path, args: &[&str]) -> String {
    command(ffmpeg)
        .args(["-hide_banner"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

/// Ask ffmpeg what it is and what it has. Empty when it cannot be run.
pub fn abilities(ffmpeg: &Path) -> Abilities {
    Abilities {
        version: ask(ffmpeg, &["-version"]).lines().next().unwrap_or("").to_string(),
        filters: parse_filters(&ask(ffmpeg, &["-filters"])),
        encoders: parse_encoders(&ask(ffmpeg, &["-encoders"])),
    }
}

/// Where the picture comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grab {
    /// A moving test picture, so everything else can run with no screen.
    Test { size: (u32, u32) },
    /// Desktop Duplication, by the monitor's place among the first
    /// graphics adapter's outputs.
    Ddagrab { output_idx: u32 },
    /// Windows.Graphics.Capture, by the monitor's handle.
    Gfxcapture { hmonitor: u64 },
    /// Windows.Graphics.Capture of one window, by its handle: the game and
    /// nothing else that is on screen. Fitted into a picture of `canvas`
    /// size whatever the window's own, so that its pieces join with those
    /// of the monitor it is on.
    Window { hwnd: u64, canvas: (u32, u32) },
    /// The old way, copying the desktop's pixels: slower, and blind to
    /// some full-screen games, but it needs no graphics card at all. By
    /// the monitor's place and size on the desktop.
    Gdigrab { origin: (i32, i32), size: (u32, u32) },
}

impl Grab {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Test { .. } => "test picture",
            Self::Ddagrab { .. } => "ddagrab",
            Self::Gfxcapture { .. } => "gfxcapture",
            Self::Window { .. } => "the game's window",
            Self::Gdigrab { .. } => "gdigrab",
        }
    }
}

/// Where the frames are when the encoder gets them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frames {
    /// Still on the graphics card: nothing is copied.
    Gpu,
    /// Downloaded to memory, which any encoder can take.
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Nvenc,
    Amf,
    Qsv,
    Software,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoder {
    pub name: &'static str,
    pub family: Family,
    pub codec: Codec,
}

/// The encoders for a codec, best first: the three graphics card makers',
/// then the one that runs on the processor.
pub fn encoders(codec: Codec) -> [Encoder; 4] {
    let names = match codec {
        Codec::H264 => ["h264_nvenc", "h264_amf", "h264_qsv", "libx264"],
        Codec::Hevc => ["hevc_nvenc", "hevc_amf", "hevc_qsv", "libx265"],
        Codec::Av1 => ["av1_nvenc", "av1_amf", "av1_qsv", "libsvtav1"],
    };
    let families = [Family::Nvenc, Family::Amf, Family::Qsv, Family::Software];
    [0, 1, 2, 3].map(|i| Encoder { name: names[i], family: families[i], codec })
}

/// One whole way of making the video: capture, where frames live, encoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Video {
    pub grab: Grab,
    pub frames: Frames,
    pub encoder: Encoder,
    pub fps: u32,
    pub cursor: bool,
    /// The size to scale to, when it is not the monitor's own.
    pub scale: Option<(u32, u32)>,
    pub bitrate_kbps: u32,
}

impl Video {
    pub fn describe(&self) -> String {
        let frames = if self.frames == Frames::Gpu { "frames kept on the GPU" } else { "frames downloaded" };
        format!("{} -> {} ({frames})", self.grab.name(), self.encoder.name)
    }

    /// The container the pieces are written in: MPEG-TS, except for AV1,
    /// which it cannot hold.
    pub fn container(&self) -> Container {
        if self.encoder.codec == Codec::Av1 { Container::FragmentedMp4 } else { Container::MpegTs }
    }

    /// The input: a filter graph read through ffmpeg's `lavfi` device.
    pub fn input(&self) -> Vec<String> {
        let onoff = |on: bool| if on { 1 } else { 0 };
        let download = if self.frames == Frames::Memory { ",hwdownload,format=bgra" } else { "" };
        if let Grab::Gdigrab { origin, size } = &self.grab {
            // Not a filter but a device of its own, with its own options.
            return [
                ("-f", "gdigrab".to_string()),
                ("-framerate", self.fps.to_string()),
                ("-draw_mouse", onoff(self.cursor).to_string()),
                ("-offset_x", origin.0.to_string()),
                ("-offset_y", origin.1.to_string()),
                ("-video_size", format!("{}x{}", size.0, size.1)),
                ("-i", "desktop".to_string()),
            ]
            .into_iter()
            .flat_map(|(key, value)| [key.to_string(), value])
            .collect();
        }
        let graph = match &self.grab {
            Grab::Gdigrab { .. } => String::new(),
            Grab::Test { size } => format!("testsrc2=size={}x{}:rate={}", size.0, size.1, self.fps),
            Grab::Ddagrab { output_idx } => {
                format!("ddagrab=output_idx={output_idx}:framerate={}:draw_mouse={}{download}", self.fps, onoff(self.cursor))
            }
            Grab::Gfxcapture { hmonitor } => {
                // This one scales on the graphics card itself.
                let size = self.scale.map(|(w, h)| format!(":width={w}:height={h}:resize_mode=scale")).unwrap_or_default();
                format!("gfxcapture=hmonitor={hmonitor}:max_framerate={}:capture_cursor={}{size}{download}", self.fps, onoff(self.cursor))
            }
            Grab::Window { hwnd, canvas } => {
                // Scaled to fit with its shape kept: a window that is not
                // the monitor's shape gets black beside it, not stretched.
                format!(
                    "gfxcapture=hwnd={hwnd}:max_framerate={}:capture_cursor={}:width={}:height={}:resize_mode=scale_aspect{download}",
                    self.fps,
                    onoff(self.cursor),
                    canvas.0,
                    canvas.1
                )
            }
        };
        let mut args = Vec::new();
        if matches!(self.grab, Grab::Test { .. }) {
            // A generated picture comes as fast as it can be made unless
            // ffmpeg is told to read it at its own rate.
            args.push("-re".to_string());
        }
        args.extend(["-f".to_string(), "lavfi".to_string(), "-i".to_string(), graph]);
        args
    }

    /// The filters between the input and the encoder, if any are needed.
    pub fn filters(&self) -> Option<String> {
        let mut chain: Vec<String> = Vec::new();
        if matches!(self.grab, Grab::Gfxcapture { .. } | Grab::Window { .. }) {
            // It gives frames as the compositor makes them; the pieces need
            // a steady rate.
            chain.push(format!("fps={}", self.fps));
        }
        match (self.frames, self.encoder.family) {
            (Frames::Gpu, Family::Qsv) => chain.extend(["hwmap=derive_device=qsv".to_string(), "format=qsv".to_string()]),
            (Frames::Gpu, _) => {}
            (Frames::Memory, family) => {
                // The screen's colours are turned into the encoder's here,
                // and said to be: as BT.709 in the limited range, by name.
                // Left to itself the conversion is BT.601 with nothing
                // saying so, and a player takes a picture this size for
                // BT.709: every colour a little off.
                let size = match (&self.grab, self.scale) {
                    (Grab::Ddagrab { .. } | Grab::Gdigrab { .. }, Some((w, h))) => format!("{w}:{h}:flags=bilinear:"),
                    _ => String::new(),
                };
                chain.push(format!("scale={size}out_color_matrix=bt709:out_range=tv"));
                chain.push(if family == Family::Software { "format=yuv420p" } else { "format=nv12" }.to_string());
                // What the encoder is told comes from the frames; the
                // conversion marks them with the matrix and range only.
                chain.push("setparams=color_primaries=bt709:color_trc=bt709".to_string());
            }
        }
        (!chain.is_empty()).then(|| chain.join(","))
    }

    /// The encoder and its settings: a constant bitrate, so the buffer is
    /// the same size whatever is on screen, and a keyframe every second
    /// with nothing reordered, so every piece starts on one.
    pub fn encode(&self) -> Vec<String> {
        let rate = format!("{}k", self.bitrate_kbps);
        let mut args = vec!["-c:v".to_string(), self.encoder.name.to_string()];
        let extra: &[&str] = match (self.encoder.family, self.encoder.codec) {
            (Family::Nvenc | Family::Amf, _) => &["-rc", "cbr"],
            (Family::Qsv, _) => &[],
            (Family::Software, Codec::Av1) => &["-preset", "12"],
            (Family::Software, _) => &["-preset", "ultrafast"],
        };
        args.extend(extra.iter().map(|s| s.to_string()));
        for (key, value) in [("-b:v", rate.clone()), ("-maxrate", rate.clone()), ("-bufsize", rate), ("-g", self.fps.to_string()), ("-bf", "0".to_string())] {
            // SVT-AV1 refuses a ceiling beside a target bitrate.
            if key == "-maxrate" && self.encoder.name == "libsvtav1" {
                continue;
            }
            args.extend([key.to_string(), value]);
        }
        if self.frames == Frames::Memory {
            // What the filters made of the picture, for an ffmpeg that
            // does not pass it on from the frames by itself. Frames kept
            // on the graphics card are converted by the encoder, which
            // says in its own way how; nothing is claimed for those here.
            args.extend(words(&["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709", "-color_range", "tv"]));
        }
        args
    }
}

/// Whether this ffmpeg's `gfxcapture` goes on giving frames while what it
/// captures gives none (a window that is minimised): the newer ones do,
/// and say so by having the option that turns it off.
pub fn capture_keeps_flowing(ffmpeg: &Path) -> bool {
    ask(ffmpeg, &["-h", "filter=gfxcapture"]).contains("dup_frames")
}

/// Every way of making the video worth trying, best first: every way that
/// keeps the frames on the graphics card, encoder by encoder, before any
/// that downloads them. Frames stay on the card only when the encoder is
/// on the card the screen is on, so on a laptop with two the encoder of
/// the one that owns the panel is found before another card's is settled
/// for with every frame copied through memory.
pub fn candidates(grabs: &[Grab], codec: Codec, scale: Option<(u32, u32)>, has: &Abilities) -> Vec<(Grab, Frames, Encoder)> {
    let mut out = Vec::new();
    for frames in [Frames::Gpu, Frames::Memory] {
        for encoder in encoders(codec).into_iter().filter(|e| has.encoders.contains(e.name)) {
            for grab in grabs {
                let fits = match (grab, frames, encoder.family) {
                    // A software encoder cannot read the graphics card.
                    (_, Frames::Gpu, Family::Software) => false,
                    // Nothing in ffmpeg scales Direct3D frames in place.
                    (Grab::Ddagrab { .. }, Frames::Gpu, _) => scale.is_none(),
                    // Copied pixels were never on the graphics card.
                    (Grab::Gdigrab { .. }, Frames::Gpu, _) => false,
                    _ => true,
                };
                if fits {
                    out.push((grab.clone(), frames, encoder));
                }
            }
        }
    }
    out
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Split the one sound stream into a stereo stream a track, in order:
/// track `n` is channels `2n` and `2n+1`.
pub fn audio_graph(tracks: usize) -> Option<String> {
    if tracks < 2 {
        return None;
    }
    let splits: String = (0..tracks).map(|n| format!("[s{n}]")).collect();
    let mut graph = format!("[1:a]asplit={tracks}{splits}");
    for n in 0..tracks {
        graph.push_str(&format!(";[s{n}]pan=stereo|c0=c{}|c1=c{}[a{n}]", 2 * n, 2 * n + 1));
    }
    Some(graph)
}

/// The recording itself: capture, encode, and cut into numbered one-second
/// pieces in the ring, starting at `start_number`. The sound of `tracks`
/// tracks is read from stdin.
///
/// The pieces keep the recording's running timestamps (no
/// `-reset_timestamps`): joined, they then line up exactly, where pieces
/// that each start at zero drift apart by a few milliseconds a join.
pub fn record_args(video: &Video, tracks: usize, ring: &Path, start_number: u64, audio_offset_ms: i64) -> Vec<String> {
    let mut args = words(&["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info", "-y"]);
    args.extend(video.input());
    if tracks > 0 {
        if audio_offset_ms != 0 {
            args.extend(["-itsoffset".to_string(), format!("{:.3}", audio_offset_ms as f64 / 1000.0)]);
        }
        args.extend(words(&["-f", "f32le", "-ar", "48000", "-ac"]));
        args.extend([(tracks * crate::mixer::CHANNELS).to_string(), "-i".to_string(), "pipe:0".to_string()]);
    }
    let graphs: Vec<String> = video.filters().map(|chain| format!("[0:v]{chain}[v]")).into_iter().chain(audio_graph(tracks)).collect();
    if !graphs.is_empty() {
        args.extend(["-filter_complex".to_string(), graphs.join(";")]);
    }
    args.extend(["-map".to_string(), if video.filters().is_some() { "[v]" } else { "0:v" }.to_string()]);
    for n in 0..tracks {
        args.extend(["-map".to_string(), if tracks == 1 { "1:a".to_string() } else { format!("[a{n}]") }]);
    }
    args.extend(video.encode());
    args.extend(["-fps_mode".to_string(), "cfr".to_string(), "-r".to_string(), video.fps.to_string()]);
    if tracks > 0 {
        args.extend(words(&["-c:a", "aac", "-b:a", "192k", "-ar", "48000"]));
    }
    let container = video.container();
    // Half a frame of leeway, so a keyframe a hair before the second still
    // starts a piece.
    let leeway = format!("{:.4}", 0.5 / video.fps as f64);
    args.extend(words(&["-f", "segment", "-segment_time", "1", "-segment_time_delta", &leeway, "-segment_format"]));
    match container {
        Container::MpegTs => args.push("mpegts".to_string()),
        Container::FragmentedMp4 => {
            // Fragments are whole as soon as they are written, like MPEG-TS.
            args.extend(words(&["mp4", "-segment_format_options", "movflags=+frag_keyframe+empty_moov+default_base_moof"]));
        }
    }
    args.extend(["-segment_start_number".to_string(), start_number.to_string()]);
    args.push(ring.join(crate::ring::pattern(container)).to_string_lossy().into_owned());
    args
}

/// A second of the real thing with the result thrown away: does this way
/// of capturing and encoding work here?
pub fn probe_args(video: &Video) -> Vec<String> {
    let mut args = words(&["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error", "-y"]);
    args.extend(video.input());
    if let Some(chain) = video.filters() {
        args.extend(["-vf".to_string(), chain]);
    }
    args.extend(video.encode());
    args.extend(words(&["-t", "1", "-f", "null", "-"]));
    args
}

/// A second of a generated picture through `encoder`: is the encoder
/// itself usable, whatever the screen is doing?
pub fn encoder_test_args(encoder: Encoder) -> Vec<String> {
    let video = Video {
        grab: Grab::Test { size: (1280, 720) },
        frames: Frames::Memory,
        encoder,
        fps: 30,
        cursor: false,
        scale: None,
        bitrate_kbps: 4000,
    };
    let mut args = probe_args(&video);
    // As fast as it goes: nothing here needs to keep time.
    args.retain(|a| a != "-re");
    args
}

/// Join the pieces named in `list` into one MP4 at `out`, copying, with the
/// index in front so it plays while still being read.
pub fn join_args(list: &Path, out: &Path, codec: Codec) -> Vec<String> {
    let mut args = words(&["-hide_banner", "-nostdin", "-nostats", "-loglevel", "warning", "-y", "-f", "concat", "-safe", "0", "-i"]);
    args.push(list.to_string_lossy().into_owned());
    args.extend(words(&["-map", "0", "-c", "copy"]));
    if codec == Codec::Hevc {
        // The tag Apple's and most other players want on H.265 in MP4.
        args.extend(words(&["-tag:v", "hvc1"]));
    }
    args.extend(words(&["-movflags", "+faststart", "-f", "mp4"]));
    args.push(out.to_string_lossy().into_owned());
    args
}

/// How long a one-second try of a way to capture may take. One that
/// gets no picture at all (a window that shows nothing new, a driver
/// that hangs) would otherwise never end, and the recorder with it.
pub const PROBE_LIMIT: Duration = Duration::from_secs(10);
/// How long joining a clip may take at the least: copying, so seconds
/// for most.
pub const JOIN_LIMIT: Duration = Duration::from_secs(120);

/// How long joining pieces of `bytes` in all may take: the least, and the
/// time to write them twice (the index is put in front once the clip is
/// whole) to a disk that manages 20 MB a second.
pub fn join_limit(bytes: u64) -> Duration {
    JOIN_LIMIT + Duration::from_secs(bytes / 10_000_000)
}

/// Run ffmpeg to its end with nothing on stdin, for `limit` at the most.
/// `Ok` when it succeeded, else the last thing it said; one that is not
/// done in time is ended.
pub fn run_quietly(ffmpeg: &Path, args: &[String], limit: Duration) -> Result<(), String> {
    let mut child = command(ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("ffmpeg could not be started: {err}"))?;
    // A try or a join outlives this program as little as the recording does.
    crate::sys::leash().hold(&child);
    // Read as it is written, so a talkative ffmpeg is never held up by a full pipe.
    let said = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stderr, &mut text);
            String::from_utf8_lossy(&text).into_owned()
        })
    });
    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if began.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let said = said.and_then(|thread| thread.join().ok()).unwrap_or_default();
    match status {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(format!("{status}: {}", gist(&said))),
        None => Err(format!("it was not done after {} s: {}", limit.as_secs(), gist(&said))),
    }
}

/// What matters in what a failed ffmpeg wrote: the first two lines, where
/// it says what went wrong, and the last, where it says what it did then.
pub fn gist(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let mut picked: Vec<&str> = lines.iter().take(2).copied().collect();
    if lines.len() > 2 {
        picked.extend(lines.last());
    }
    if picked.is_empty() { "it said nothing".to_string() } else { picked.join(" / ") }
}

/// The words of a command as one line for the log.
pub fn shown(program: &Path, args: &[String]) -> String {
    let quote = |word: &str| if word.contains([' ', '|', ';', '[']) { format!("\"{word}\"") } else { word.to_string() };
    std::iter::once(program.to_string_lossy().into_owned()).chain(args.iter().map(|a| quote(a))).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(grab: Grab, frames: Frames, encoder: Encoder) -> Video {
        Video { grab, frames, encoder, fps: 60, cursor: true, scale: None, bitrate_kbps: 20000 }
    }

    fn enc(codec: Codec, family: Family) -> Encoder {
        encoders(codec).into_iter().find(|e| e.family == family).unwrap()
    }

    fn all_abilities() -> Abilities {
        let encoders = Codec::ALL.iter().flat_map(|&c| encoders(c)).map(|e| e.name.to_string()).collect();
        Abilities { version: String::new(), filters: HashSet::new(), encoders }
    }

    /// The colours of a picture in memory turned into BT.709, limited range.
    const TO_709: &str = "scale=out_color_matrix=bt709:out_range=tv";
    const SAID_709: &str = "setparams=color_primaries=bt709:color_trc=bt709";
    const TAGS_709: &str = "-color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv";

    const DDA: Grab = Grab::Ddagrab { output_idx: 1 };
    const GFX: Grab = Grab::Gfxcapture { hmonitor: 65537 };

    #[test]
    fn ffmpegs_lists_are_read() {
        let filters = parse_filters(
            "Filters:\n  T.. = Timeline support\n  .S. = Slice threading\n ... ddagrab           |->V       Grab Windows Desktop images using Desktop Duplication API\n .. gfxcapture        |->V       Capture graphics/screen content as a video source\n T.. scale             V->V       Scale the input video size.\n",
        );
        assert!(filters.contains("ddagrab") && filters.contains("gfxcapture") && filters.contains("scale"));
        assert_eq!(filters.len(), 3);
        let encoders = parse_encoders(
            "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)\n V..... h264_qsv             H.264 (Intel Quick Sync)\n A....D aac                  AAC (Advanced Audio Coding)\n",
        );
        assert_eq!(encoders, ["h264_nvenc", "h264_qsv", "aac"].into_iter().map(String::from).collect());
    }

    #[test]
    fn desktop_duplication_into_nvenc_stays_on_the_gpu() {
        let v = video(DDA, Frames::Gpu, enc(Codec::H264, Family::Nvenc));
        assert_eq!(v.input(), ["-f", "lavfi", "-i", "ddagrab=output_idx=1:framerate=60:draw_mouse=1"]);
        assert_eq!(v.filters(), None);
        assert_eq!(v.encode().join(" "), "-c:v h264_nvenc -rc cbr -b:v 20000k -maxrate 20000k -bufsize 20000k -g 60 -bf 0");
        assert_eq!(
            probe_args(&v).join(" "),
            "-hide_banner -nostdin -nostats -loglevel error -y -f lavfi -i ddagrab=output_idx=1:framerate=60:draw_mouse=1 \
             -c:v h264_nvenc -rc cbr -b:v 20000k -maxrate 20000k -bufsize 20000k -g 60 -bf 0 -t 1 -f null -"
        );
    }

    #[test]
    fn a_software_encoder_gets_downloaded_frames() {
        let mut v = video(DDA, Frames::Memory, enc(Codec::H264, Family::Software));
        v.cursor = false;
        assert_eq!(v.input()[3], "ddagrab=output_idx=1:framerate=60:draw_mouse=0,hwdownload,format=bgra");
        assert_eq!(v.filters().as_deref(), Some(&*format!("{TO_709},format=yuv420p,{SAID_709}")));
        assert!(v.encode().join(" ").starts_with("-c:v libx264 -preset ultrafast -b:v 20000k"));
        // A smaller picture is scaled once it is in memory.
        v.scale = Some((1920, 804));
        assert_eq!(v.filters().as_deref(), Some(&*format!("scale=1920:804:flags=bilinear:out_color_matrix=bt709:out_range=tv,format=yuv420p,{SAID_709}")));
        // A hardware encoder fed from memory takes NV12.
        v.encoder = enc(Codec::H264, Family::Amf);
        assert_eq!(v.filters().as_deref(), Some(&*format!("scale=1920:804:flags=bilinear:out_color_matrix=bt709:out_range=tv,format=nv12,{SAID_709}")));
    }

    #[test]
    fn graphics_capture_names_the_monitor_and_is_given_a_steady_rate() {
        let mut v = video(GFX, Frames::Gpu, enc(Codec::Hevc, Family::Amf));
        assert_eq!(v.input()[3], "gfxcapture=hmonitor=65537:max_framerate=60:capture_cursor=1");
        assert_eq!(v.filters().as_deref(), Some("fps=60"));
        assert!(v.encode().join(" ").starts_with("-c:v hevc_amf -rc cbr "));
        // It scales by itself, on the graphics card.
        v.scale = Some((1280, 720));
        assert_eq!(v.input()[3], "gfxcapture=hmonitor=65537:max_framerate=60:capture_cursor=1:width=1280:height=720:resize_mode=scale");
        assert_eq!(v.filters().as_deref(), Some("fps=60"));
        v.frames = Frames::Memory;
        v.encoder = enc(Codec::Hevc, Family::Software);
        assert!(v.input()[3].ends_with(":resize_mode=scale,hwdownload,format=bgra"));
        assert_eq!(v.filters().as_deref(), Some(&*format!("fps=60,{TO_709},format=yuv420p,{SAID_709}")));
    }

    #[test]
    fn a_games_window_is_named_by_its_handle_and_fitted_into_the_monitors_size() {
        let grab = Grab::Window { hwnd: 1312904, canvas: (2560, 1440) };
        let mut v = video(grab.clone(), Frames::Gpu, enc(Codec::H264, Family::Nvenc));
        assert_eq!(v.input(), ["-f", "lavfi", "-i", "gfxcapture=hwnd=1312904:max_framerate=60:capture_cursor=1:width=2560:height=1440:resize_mode=scale_aspect"]);
        assert_eq!(v.filters().as_deref(), Some("fps=60"), "it gives frames as the game makes them");
        assert_eq!(v.describe(), "the game's window -> h264_nvenc (frames kept on the GPU)");
        v.frames = Frames::Memory;
        v.encoder = enc(Codec::H264, Family::Software);
        assert!(v.input()[3].ends_with(":resize_mode=scale_aspect,hwdownload,format=bgra"));
        assert_eq!(v.filters().as_deref(), Some(&*format!("fps=60,{TO_709},format=yuv420p,{SAID_709}")));
        // On the graphics card for its encoders, downloaded for the rest.
        let tries = candidates(std::slice::from_ref(&grab), Codec::H264, None, &all_abilities());
        let shown: Vec<String> = tries.iter().map(|(_, f, e)| format!("{f:?} {}", e.name)).collect();
        assert_eq!(shown, ["Gpu h264_nvenc", "Gpu h264_amf", "Gpu h264_qsv", "Memory h264_nvenc", "Memory h264_amf", "Memory h264_qsv", "Memory libx264"]);
    }

    #[test]
    fn quick_sync_maps_the_frames_to_its_own_device() {
        let v = video(DDA, Frames::Gpu, enc(Codec::H264, Family::Qsv));
        assert_eq!(v.filters().as_deref(), Some("hwmap=derive_device=qsv,format=qsv"));
        assert_eq!(v.encode().join(" "), "-c:v h264_qsv -b:v 20000k -maxrate 20000k -bufsize 20000k -g 60 -bf 0");
    }

    #[test]
    fn copied_pixels_name_the_monitors_place_and_reach_any_encoder() {
        let grab = Grab::Gdigrab { origin: (-1920, 0), size: (1920, 1080) };
        let mut v = video(grab.clone(), Frames::Memory, enc(Codec::H264, Family::Software));
        assert_eq!(v.input().join(" "), "-f gdigrab -framerate 60 -draw_mouse 1 -offset_x -1920 -offset_y 0 -video_size 1920x1080 -i desktop");
        assert_eq!(v.filters().as_deref(), Some(&*format!("{TO_709},format=yuv420p,{SAID_709}")));
        v.scale = Some((1280, 720));
        assert_eq!(v.filters().as_deref(), Some(&*format!("scale=1280:720:flags=bilinear:out_color_matrix=bt709:out_range=tv,format=yuv420p,{SAID_709}")));
        // Never as frames on the graphics card, and after the better ways.
        let tries = candidates(&[DDA, grab.clone()], Codec::H264, None, &all_abilities());
        assert!(tries.iter().all(|(g, frames, _)| *g != grab || *frames == Frames::Memory));
        assert_eq!(tries.last().map(|(g, _, e)| (g.clone(), e.name)), Some((grab, "libx264")));
    }

    #[test]
    fn the_test_picture_is_read_at_its_own_rate() {
        let mut v = video(Grab::Test { size: (320, 180) }, Frames::Memory, enc(Codec::H264, Family::Software));
        v.fps = 30;
        assert_eq!(v.input(), ["-re", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30"]);
        assert_eq!(v.filters().as_deref(), Some(&*format!("{TO_709},format=yuv420p,{SAID_709}")));
    }

    #[test]
    fn the_tries_go_from_best_to_surest() {
        let tries = candidates(&[DDA, GFX], Codec::H264, None, &all_abilities());
        let shown: Vec<String> = tries.iter().map(|(g, f, e)| format!("{} {:?} {}", g.name(), f, e.name)).collect();
        assert_eq!(
            shown,
            [
                "ddagrab Gpu h264_nvenc",
                "gfxcapture Gpu h264_nvenc",
                "ddagrab Gpu h264_amf",
                "gfxcapture Gpu h264_amf",
                "ddagrab Gpu h264_qsv",
                "gfxcapture Gpu h264_qsv",
                "ddagrab Memory h264_nvenc",
                "gfxcapture Memory h264_nvenc",
                "ddagrab Memory h264_amf",
                "gfxcapture Memory h264_amf",
                "ddagrab Memory h264_qsv",
                "gfxcapture Memory h264_qsv",
                "ddagrab Memory libx264",
                "gfxcapture Memory libx264",
            ]
        );
    }

    #[test]
    fn no_frames_are_downloaded_while_some_encoder_may_take_them_on_the_card() {
        // A laptop whose panel is on the Intel card: NVENC cannot take its
        // frames as they are, Quick Sync can, and is tried before NVENC is
        // settled for with every frame copied through memory.
        let pixels = Grab::Gdigrab { origin: (0, 0), size: (1920, 1080) };
        for codec in Codec::ALL {
            let tries = candidates(&[DDA, GFX, pixels.clone()], codec, None, &all_abilities());
            let first_download = tries.iter().position(|(_, frames, _)| *frames == Frames::Memory).unwrap();
            assert!(tries[first_download..].iter().all(|(_, frames, _)| *frames == Frames::Memory), "{codec:?}");
            let on_the_card: Vec<Family> = tries[..first_download].iter().map(|(_, _, e)| e.family).collect();
            assert_eq!(on_the_card, [Family::Nvenc, Family::Nvenc, Family::Amf, Family::Amf, Family::Qsv, Family::Qsv], "{codec:?}");
            // No more tries than before, and the surest still last.
            assert_eq!(tries.len(), 6 + 3 * 3 + 3);
            assert_eq!(tries.last().map(|(g, _, e)| (g.clone(), e.family)), Some((pixels.clone(), Family::Software)));
        }
    }

    #[test]
    fn a_picture_converted_in_memory_is_bt709_and_says_so() {
        let v = video(DDA, Frames::Memory, enc(Codec::H264, Family::Nvenc));
        assert_eq!(v.filters().as_deref(), Some(&*format!("{TO_709},format=nv12,{SAID_709}")));
        assert!(v.encode().join(" ").ends_with(&format!("-g 60 -bf 0 {TAGS_709}")));
        // Frames kept on the card are converted by the encoder: nothing is
        // put in their way, and nothing claimed for them.
        let v = video(DDA, Frames::Gpu, enc(Codec::H264, Family::Nvenc));
        assert_eq!(v.filters(), None);
        assert!(!v.encode().join(" ").contains("color"));
    }

    #[test]
    fn a_long_clip_is_given_longer_to_be_joined() {
        assert_eq!(join_limit(0), JOIN_LIMIT);
        assert_eq!(join_limit(450_000_000), JOIN_LIMIT + Duration::from_secs(45));
        // Ten minutes at 100 Mbit/s.
        assert!(join_limit(7_500_000_000) > Duration::from_secs(600));
    }

    #[test]
    fn the_recording_ffmpeg_is_another_command_than_the_rest() {
        // The priority itself is Windows' to give; the program is the same.
        assert_eq!(recording_command(Path::new("ffmpeg")).get_program(), command(Path::new("ffmpeg")).get_program());
    }

    #[test]
    fn a_smaller_picture_is_not_tried_where_nothing_can_scale_it() {
        let tries = candidates(&[DDA, GFX], Codec::Hevc, Some((1280, 720)), &all_abilities());
        assert!(!tries.iter().any(|(g, f, _)| *g == DDA && *f == Frames::Gpu));
        assert_eq!(tries[0], (GFX, Frames::Gpu, enc(Codec::Hevc, Family::Nvenc)));
        assert_eq!(tries[1], (GFX, Frames::Gpu, enc(Codec::Hevc, Family::Amf)));
        let first_download = tries.iter().find(|(_, frames, _)| *frames == Frames::Memory);
        assert_eq!(first_download, Some(&(DDA, Frames::Memory, enc(Codec::Hevc, Family::Nvenc))));
    }

    #[test]
    fn encoders_this_ffmpeg_lacks_are_not_tried() {
        let mut has = Abilities::default();
        has.encoders.insert("libx264".into());
        let tries = candidates(&[DDA], Codec::H264, None, &has);
        assert_eq!(tries, [(DDA, Frames::Memory, enc(Codec::H264, Family::Software))]);
        assert!(candidates(&[DDA], Codec::Av1, None, &has).is_empty());
    }

    #[test]
    fn the_sound_is_split_into_a_stereo_stream_a_track() {
        assert_eq!(audio_graph(0), None);
        assert_eq!(audio_graph(1), None, "one track is the stream as it is");
        assert_eq!(
            audio_graph(3).unwrap(),
            "[1:a]asplit=3[s0][s1][s2];[s0]pan=stereo|c0=c0|c1=c1[a0];[s1]pan=stereo|c0=c2|c1=c3[a1];[s2]pan=stereo|c0=c4|c1=c5[a2]"
        );
    }

    #[test]
    fn the_recording_line_for_windows_with_two_tracks() {
        let v = video(DDA, Frames::Gpu, enc(Codec::H264, Family::Nvenc));
        let line = record_args(&v, 2, Path::new("ring"), 17, 0).join(" ");
        assert_eq!(
            line,
            "-hide_banner -nostdin -nostats -loglevel info -y \
             -f lavfi -i ddagrab=output_idx=1:framerate=60:draw_mouse=1 \
             -f f32le -ar 48000 -ac 4 -i pipe:0 \
             -filter_complex [1:a]asplit=2[s0][s1];[s0]pan=stereo|c0=c0|c1=c1[a0];[s1]pan=stereo|c0=c2|c1=c3[a1] \
             -map 0:v -map [a0] -map [a1] \
             -c:v h264_nvenc -rc cbr -b:v 20000k -maxrate 20000k -bufsize 20000k -g 60 -bf 0 -fps_mode cfr -r 60 \
             -c:a aac -b:a 192k -ar 48000 \
             -f segment -segment_time 1 -segment_time_delta 0.0083 -segment_format mpegts -segment_start_number 17 "
                .to_string()
                + &Path::new("ring").join("piece_%09d.ts").to_string_lossy()
        );
    }

    #[test]
    fn the_recording_line_for_the_software_path_with_one_track() {
        let v = video(DDA, Frames::Memory, enc(Codec::H264, Family::Software));
        let line = record_args(&v, 1, Path::new("ring"), 0, -80).join(" ");
        assert!(line.contains("-i ddagrab=output_idx=1:framerate=60:draw_mouse=1,hwdownload,format=bgra -itsoffset -0.080 -f f32le -ar 48000 -ac 2 -i pipe:0"), "{line}");
        assert!(line.contains(&format!("-filter_complex [0:v]{TO_709},format=yuv420p,{SAID_709}[v] -map [v] -map 1:a -c:v libx264 -preset ultrafast")), "{line}");
        assert!(line.contains(&format!("-bf 0 {TAGS_709} -fps_mode cfr -r 60")), "{line}");
    }

    #[test]
    fn the_recording_line_with_no_sound_and_av1() {
        let v = video(GFX, Frames::Gpu, enc(Codec::Av1, Family::Nvenc));
        let line = record_args(&v, 0, Path::new("ring"), 0, 0).join(" ");
        assert!(!line.contains("pipe:0") && !line.contains("-c:a"), "{line}");
        assert!(line.contains("-filter_complex [0:v]fps=60[v] -map [v] -c:v av1_nvenc"), "{line}");
        assert!(
            line.contains("-segment_format mp4 -segment_format_options movflags=+frag_keyframe+empty_moov+default_base_moof -segment_start_number 0"),
            "AV1 cannot go in MPEG-TS: {line}"
        );
        assert!(line.ends_with("piece_%09d.mp4"), "{line}");
    }

    #[test]
    fn the_test_recording_line() {
        let mut v = video(Grab::Test { size: (320, 180) }, Frames::Memory, enc(Codec::H264, Family::Software));
        v.fps = 30;
        v.bitrate_kbps = 1000;
        let line = record_args(&v, 2, Path::new("r"), 0, 0).join(" ");
        assert!(line.starts_with("-hide_banner -nostdin -nostats -loglevel info -y -re -f lavfi -i testsrc2=size=320x180:rate=30 -f f32le"), "{line}");
        assert!(line.contains(&format!("-filter_complex [0:v]{TO_709},format=yuv420p,{SAID_709}[v];[1:a]asplit=2")), "{line}");
        assert!(line.contains(&format!("-g 30 -bf 0 {TAGS_709} -fps_mode cfr -r 30")), "{line}");
    }

    #[test]
    fn a_clip_is_joined_by_copying() {
        let line = join_args(Path::new("clip.txt"), Path::new("Replay.mp4.part"), Codec::H264).join(" ");
        assert_eq!(line, "-hide_banner -nostdin -nostats -loglevel warning -y -f concat -safe 0 -i clip.txt -map 0 -c copy -movflags +faststart -f mp4 Replay.mp4.part");
        let hevc = join_args(Path::new("clip.txt"), Path::new("o"), Codec::Hevc).join(" ");
        assert!(hevc.contains("-c copy -tag:v hvc1 -movflags"), "{hevc}");
    }

    #[test]
    fn an_encoder_is_tried_on_a_generated_picture() {
        let line = encoder_test_args(enc(Codec::Hevc, Family::Qsv)).join(" ");
        assert_eq!(
            line,
            format!(
                "-hide_banner -nostdin -nostats -loglevel error -y -f lavfi -i testsrc2=size=1280x720:rate=30 -vf {TO_709},format=nv12,{SAID_709} \
                 -c:v hevc_qsv -b:v 4000k -maxrate 4000k -bufsize 4000k -g 30 -bf 0 {TAGS_709} -t 1 -f null -"
            )
        );
    }

    #[test]
    fn the_software_av1_encoder_gets_no_ceiling() {
        let v = video(DDA, Frames::Memory, enc(Codec::Av1, Family::Software));
        assert_eq!(v.encode().join(" "), format!("-c:v libsvtav1 -preset 12 -b:v 20000k -bufsize 20000k -g 60 -bf 0 {TAGS_709}"));
    }

    #[test]
    fn a_failure_is_told_by_its_first_and_last_lines() {
        assert_eq!(gist("[ddagrab] no output 3\n\nfailed to open\nmore\nExiting\n"), "[ddagrab] no output 3 / failed to open / Exiting");
        assert_eq!(gist("only this\n"), "only this");
        assert_eq!(gist(""), "it said nothing");
    }

    #[test]
    fn an_ffmpeg_that_is_not_done_in_time_is_ended() {
        if !Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            // Where the tests are run to prove something, a skip proves nothing.
            assert!(std::env::var("ZENITH_REQUIRE_FFMPEG").as_deref() != Ok("1"), "ffmpeg is not installed, and ZENITH_REQUIRE_FFMPEG=1 says it has to be");
            println!("NOTE: skipped: ffmpeg is not installed");
            return;
        }
        // A picture read at its own rate, for an hour.
        let endless = words(&["-hide_banner", "-nostdin", "-re", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5", "-t", "3600", "-f", "null", "-"]);
        let began = Instant::now();
        let err = run_quietly(Path::new("ffmpeg"), &endless, Duration::from_millis(600)).unwrap_err();
        assert!(err.contains("not done"), "{err}");
        assert!(began.elapsed() < Duration::from_secs(10));
        let quick = words(&["-hide_banner", "-nostdin", "-f", "lavfi", "-i", "testsrc2=size=64x64:rate=5", "-t", "1", "-f", "null", "-"]);
        assert_eq!(run_quietly(Path::new("ffmpeg"), &quick, PROBE_LIMIT), Ok(()));
        assert!(run_quietly(Path::new("ffmpeg"), &words(&["-i", "no such file"]), PROBE_LIMIT).is_err());
    }

    #[test]
    fn a_command_is_shown_so_it_can_be_pasted() {
        let args = vec!["-i".to_string(), "a b".to_string(), "-map".to_string(), "[v]".to_string()];
        assert_eq!(shown(Path::new("ffmpeg"), &args), "ffmpeg -i \"a b\" -map \"[v]\"");
    }
}
