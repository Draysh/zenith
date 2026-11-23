//! The recording: ffmpeg kept running and fed, its ring kept to size, and
//! the words on stdin carried out. `save` joins the newest pieces into a
//! clip and prints its path, or `save-failed: ` and why there is no clip;
//! `stop`, or stdin ending because the daemon is gone, ends the recording.
//!
//! What is recorded is the monitor `-w` names, until a game's window is on
//! screen: then that window alone is, for as long as it is there. Started
//! because a game was (`-wait-for-game`), nothing is recorded until that
//! window first appears: the desktop while a game starts up is no clip. Each
//! change is a new ffmpeg writing on into the same ring, in the same
//! encoding and size, so a clip that spans one still joins. When the size
//! or the encoder has to change (the monitor was given another mode), the
//! ring starts over: a clip never joins pieces of two kinds.

use crate::args::{Codec, Options};
use crate::audio::processes::{self, ProcessTable};
use crate::ffmpeg::{self, Abilities, Encoder, Family, Frames, Grab, Video};
use crate::game_window::{self, Rules, Window};
use crate::mixer::{self, Sink};
use crate::monitors::Monitor;
use crate::sys::ChildLeash;
use crate::{monitors, note, ring, sys};
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the ring and ffmpeg are looked at between words on stdin.
const LOOK_EVERY: Duration = Duration::from_millis(200);
/// How long a save waits for the piece being written to be finished.
const PIECE_WAIT: Duration = Duration::from_millis(2500);
/// No new piece for this long means ffmpeg is stuck, and it is started afresh.
const STALL: Duration = Duration::from_secs(12);
/// An ffmpeg that ran this long is counted as having started properly.
const HEALTHY_AFTER: Duration = Duration::from_secs(10);
/// After this many short runs in a row, give up: the daemon reports it.
const MAX_SHORT_RUNS: u32 = 5;
/// MPEG-TS timestamps wrap after 26 hours; ffmpeg is started afresh well
/// before, which costs a second of the buffer once in a very long session.
const MAX_RUN: Duration = Duration::from_secs(20 * 3600);
/// How often the windows are looked through for a game's.
const WINDOWS_EVERY: Duration = Duration::from_secs(1);
/// How long another window (or none) has to be the one to record before
/// it is: a game starting goes through a few windows on its way, and each
/// change costs a moment of the buffer.
const SETTLE: Duration = Duration::from_millis(1500);
/// A window that would not be recorded is left alone this long, and
/// twice as long each time it would not again.
const LEAVE_ALONE: Duration = Duration::from_secs(60);
/// No new piece from a window for this long, with an ffmpeg that does not
/// fill in for a window that shows nothing new: the monitor is recorded
/// instead, sooner than `STALL` would have it.
const WINDOW_STALL: Duration = Duration::from_secs(5);

/// How long the way of recording the monitor that was settled for is kept
/// before the one this recording began with is tried again, the first
/// time: the lock screen and the like, where only the copying of pixels
/// works, do not last.
const BETTER_AFTER: Duration = Duration::from_secs(5 * 60);
/// Each try is made twice as long after the last, up to this: a way that
/// only seems to work again costs a moment of the buffer every time.
const BETTER_AT_MOST: Duration = Duration::from_secs(3600);
/// How often the room left on the ring's disk is looked at.
const DISK_EVERY: Duration = Duration::from_secs(5);
/// What a track of sound comes to in the pieces, in kbit/s.
const SOUND_KBPS: u64 = 192;
/// The most the ring may come to. More than this was not meant: a day of
/// buffer asked for by mistake would otherwise fill the disk before
/// anything said so.
pub const MAX_RING_BYTES: u64 = 32 << 30;
/// What has to be left on the ring's disk beside a full ring for the
/// recording to begin.
pub const ROOM_TO_SPARE: u64 = 256 << 20;
/// With less than this left on the ring's disk, the recording ends: the
/// next pieces would not fit, and ffmpeg would only be started over and
/// over.
pub const LEAST_ROOM: u64 = 64 << 20;
/// How every reason for ending that is about the disk begins: the daemon
/// can tell it from any other by this, and by the exit code.
pub const NO_ROOM: &str = "no room on the disk";
/// The exit code of a recording that ended for want of room on the disk.
pub const NO_ROOM_CODE: u8 = 3;
/// How the line on stdout begins that says a clip could not be saved, in
/// place of the clip's path: the reason follows it.
pub const SAVE_FAILED: &str = "save-failed: ";

/// How long the game's window is waited for before anything is recorded
/// (`-wait-for-game`). A game whose window never shows as one is then
/// recorded as the monitor, as before, so there is still something to clip.
const WINDOW_WAIT: Duration = Duration::from_secs(120);

/// What stdin asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Word {
    Save,
    Stop,
}

/// Read one line of stdin. Anything unknown is ignored (and logged).
pub fn word(line: &str) -> Option<Word> {
    match line.trim() {
        "save" => Some(Word::Save),
        "stop" | "quit" | "exit" => Some(Word::Stop),
        _ => None,
    }
}

/// The file name of a clip saved at `now`, one not in `taken`.
pub fn clip_name(now: chrono::NaiveDateTime, taken: impl Fn(&str) -> bool) -> String {
    let base = format!("Replay_{}", now.format("%Y-%m-%d_%H-%M-%S"));
    let mut name = format!("{base}.mp4");
    let mut n = 1;
    while taken(&name) {
        n += 1;
        name = format!("{base}_{n}.mp4");
    }
    name
}

/// How long to wait before starting ffmpeg again after `short_runs` runs
/// in a row that ended early.
pub fn restart_delay(short_runs: u32) -> Duration {
    Duration::from_millis(500 * short_runs.clamp(1, 8) as u64)
}

/// The line on stdout for a clip that could not be saved. Never a path, so
/// a reader that only knows paths passes over it.
pub fn save_failed_line(why: &str) -> String {
    format!("{SAVE_FAILED}{}", why.replace(['\n', '\r'], " "))
}

/// The clips in `dir` that were being written when a recorder was ended:
/// `Replay_….mp4.part`, which no finished clip is ever called.
pub fn stale_partials(dir: &Path) -> Vec<PathBuf> {
    let ours = |name: &str| name.starts_with("Replay_") && name.ends_with(".mp4.part");
    let mut found: Vec<PathBuf> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|entry| ours(&entry.file_name().to_string_lossy())).map(|entry| entry.path()).collect();
    found.sort();
    found
}

/// What the ring comes to on disk when it is full, in bytes: the picture
/// and each track of sound for the buffer and the pieces kept beyond it,
/// and a fifth more for what the container adds and the encoder overshoots.
pub fn ring_bytes(bitrate_kbps: u32, tracks: usize, buffer_secs: u32) -> u64 {
    let a_second = (bitrate_kbps as u64 + tracks as u64 * SOUND_KBPS) * 1000 / 8;
    a_second * (buffer_secs as u64 + ring::SPARE as u64) * 6 / 5
}

fn megabytes(bytes: u64) -> u64 {
    bytes / 1_000_000
}

/// Whether a ring of `ring` bytes may be begun with `free` bytes left on
/// its disk (`None`: it cannot be told), and if not, why.
pub fn room_to_begin(ring: u64, free: Option<u64>) -> Result<(), String> {
    if ring > MAX_RING_BYTES {
        return Err(format!("{NO_ROOM} for a buffer this long: it would come to {} MB, and {} MB is the most one may", megabytes(ring), megabytes(MAX_RING_BYTES)));
    }
    match free {
        Some(free) if free < ring + ROOM_TO_SPARE => {
            Err(format!("{NO_ROOM} for the replay buffer: it needs about {} MB and {} MB are free", megabytes(ring + ROOM_TO_SPARE), megabytes(free)))
        }
        _ => Ok(()),
    }
}

/// Whether a recording may go on with `free` bytes left on its ring's disk.
pub fn room_to_go_on(free: Option<u64>) -> Result<(), String> {
    match free {
        Some(free) if free < LEAST_ROOM => Err(format!("{NO_ROOM} the replay buffer is on: {} MB are left", megabytes(free))),
        _ => Ok(()),
    }
}

/// Whether a clip of `bytes` fits where `free` bytes are left.
pub fn room_for_clip(bytes: u64, free: Option<u64>) -> Result<(), String> {
    // The index and the container's own words, beside the pieces.
    let needed = bytes + bytes / 20 + (8 << 20);
    match free {
        Some(free) if free < needed => Err(format!("{NO_ROOM} for the clip: it needs about {} MB and {} MB are free", megabytes(needed), megabytes(free))),
        _ => Ok(()),
    }
}

/// Whether two ways of making the video are the same way, whatever
/// monitor or size they are of.
fn same_way(a: &Video, b: &Video) -> bool {
    a.grab.name() == b.grab.name() && a.frames == b.frames && a.encoder == b.encoder
}

/// How long after a try of the better way the next one is made.
pub fn next_better_wait(wait: Duration) -> Duration {
    (wait * 2).min(BETTER_AT_MOST)
}

/// `screen`, the way a monitor is recorded, for `monitor` as it is now:
/// its place, handle and size may have changed since the way was chosen (a
/// game gave it another mode). `wanted` is the size asked for with `-s`.
/// With it, the size of the picture. `None` when the way cannot be kept
/// and another has to be chosen.
fn screen_now(screen: &Video, monitor: &Monitor, wanted: Option<(u32, u32)>) -> Option<(Video, (u32, u32))> {
    let scale = wanted.filter(|size| *size != monitor.size);
    let grab = match &screen.grab {
        // Now on another graphics card, when it has no place on the first.
        Grab::Ddagrab { .. } => Grab::Ddagrab { output_idx: monitor.output_idx? },
        Grab::Gfxcapture { .. } => Grab::Gfxcapture { hmonitor: monitor.handle },
        Grab::Gdigrab { .. } => Grab::Gdigrab { origin: monitor.origin, size: monitor.size },
        // Not of a monitor.
        Grab::Test { .. } | Grab::Window { .. } => return None,
    };
    // Nothing in ffmpeg scales Direct3D frames in place.
    if matches!(grab, Grab::Ddagrab { .. }) && screen.frames == Frames::Gpu && scale.is_some() {
        return None;
    }
    Some((Video { grab, scale, ..screen.clone() }, scale.unwrap_or(monitor.size)))
}

/// A test recording: the test picture through the software encoder of the
/// codec asked for (libx264 when this ffmpeg has no other), so that every
/// kind of piece the ring can hold is made for real in the tests.
fn test_video(options: &Options, has: &Abilities) -> Video {
    let software = |codec| ffmpeg::encoders(codec).into_iter().find(|e| e.family == Family::Software);
    let encoder = software(options.codec)
        .filter(|e| has.encoders.contains(e.name))
        .or_else(|| software(Codec::H264))
        .unwrap_or(Encoder { name: "libx264", family: Family::Software, codec: Codec::H264 });
    Video {
        grab: Grab::Test { size: options.size.unwrap_or((1280, 720)) },
        frames: Frames::Memory,
        encoder,
        fps: options.fps,
        cursor: options.cursor,
        scale: None,
        bitrate_kbps: options.bitrate_kbps,
    }
}

/// A way of recording the monitor that works.
struct Chosen {
    video: Video,
    /// The size of the picture it makes.
    canvas: (u32, u32),
    /// The name of the monitor it is of: `-w screen` may mean another one
    /// tomorrow, and this recording stays with the one it began on.
    monitor: String,
}

/// Try each way of capturing and encoding for a second and take the first
/// that works. A codec nothing here can encode gives way to H.264: a clip
/// in another codec is better than no clip.
fn choose_video(ffmpeg: &Path, has: &Abilities, options: &Options) -> Result<Chosen, String> {
    if monitors::test_source() {
        note!("test picture in place of a screen, encoded in software");
        let video = test_video(options, has);
        let Grab::Test { size } = video.grab else { return Err("the test picture has no size".into()) };
        return Ok(Chosen { video, canvas: size, monitor: monitors::TEST_NAME.to_string() });
    }
    let all = monitors::list();
    for m in &all {
        note!("monitor {} {}x{} primary={} ddagrab output={:?} handle={}", m.name, m.size.0, m.size.1, m.primary, m.output_idx, m.handle);
    }
    let monitor = monitors::pick(&options.window, &all).ok_or_else(|| format!("there is no monitor called {}", options.window))?;
    let only = std::env::var("ZENITH_REC_CAPTURE").ok();
    let grabs = monitors::grabs(monitor, &has.filters, only.as_deref());
    if grabs.is_empty() {
        return Err(format!(
            "this ffmpeg cannot capture {} (ddagrab: {}, gfxcapture: {}, on the first adapter: {})",
            monitor.name,
            has.filters.contains("ddagrab"),
            has.filters.contains("gfxcapture"),
            monitor.output_idx.is_some()
        ));
    }
    let scale = options.size.filter(|size| *size != monitor.size);
    let mut codecs = vec![options.codec];
    if options.codec != Codec::H264 {
        codecs.push(Codec::H264);
    }
    for codec in codecs {
        for (grab, frames, encoder) in ffmpeg::candidates(&grabs, codec, scale, has) {
            let video = Video { grab, frames, encoder, fps: options.fps, cursor: options.cursor, scale, bitrate_kbps: options.bitrate_kbps };
            let began = Instant::now();
            match ffmpeg::run_quietly(ffmpeg, &ffmpeg::probe_args(&video), ffmpeg::PROBE_LIMIT) {
                Ok(()) => {
                    note!("{} works ({} ms)", video.describe(), began.elapsed().as_millis());
                    return Ok(Chosen { video, canvas: scale.unwrap_or(monitor.size), monitor: monitor.name.clone() });
                }
                Err(err) => note!("{} does not work here: {err}", video.describe()),
            }
        }
        note!("nothing here records {} as {}", monitor.name, codec.as_str());
    }
    Err(format!("no way of capturing and encoding {} works on this machine", monitor.name))
}

/// A running ffmpeg and what is known about it.
struct Running {
    child: Child,
    started: Instant,
    /// The last lines it wrote, for when it ends.
    tail: Arc<Mutex<VecDeque<String>>>,
}

/// Copy ffmpeg's stderr into the log. A chatty ffmpeg is thinned out after
/// its first lines, errors aside, so the log stays readable.
fn follow(stderr: std::process::ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    std::thread::spawn(move || {
        let mut seen = 0u64;
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            // One of these a second, for every piece: noise.
            if line.contains("] Opening '") {
                continue;
            }
            seen += 1;
            if seen <= 400 || seen.is_multiple_of(200) || line.to_lowercase().contains("error") {
                note!("ffmpeg: {line}");
            }
            if let Ok(mut tail) = tail.lock() {
                if tail.len() == 5 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
    });
}

/// Following the game's window: what is known for choosing it, and what
/// was learnt trying.
struct Follow {
    rules: Rules,
    /// Where the frames of a window have to be for the encoder, once a
    /// window was tried.
    frames: Option<Frames>,
    /// This ffmpeg goes on giving frames for a minimised window.
    keeps_flowing: bool,
    looked: Instant,
    /// What would be recorded in place of what is, and since when.
    pending: Option<(Option<u64>, Instant)>,
    /// Windows that would not be recorded, and until when each is left alone.
    refused: Vec<(u64, Instant)>,
    /// How many times each window would not be recorded.
    failures: Vec<(u64, u32)>,
}

impl Follow {
    /// Whether the game's window can be followed at all, and if not, why.
    fn possible(options: &Options, has: &Abilities, only: Option<&str>) -> Result<Rules, &'static str> {
        let rules = Rules::new(&options.game_dirs, &options.game_programs);
        if rules.is_empty() {
            Err("nothing was said about which programs are games")
        } else if monitors::test_source() {
            Err("the test picture is recorded")
        } else if !has.filters.contains("gfxcapture") {
            Err("this ffmpeg has no gfxcapture, the one way it has of recording a window")
        } else if only.is_some_and(|only| only != "gfxcapture") {
            Err("ZENITH_REC_CAPTURE asks for another way of capturing")
        } else {
            Ok(rules)
        }
    }

    /// The window to record now, of those on screen; `None` for the monitor.
    fn wanted<'a>(&mut self, windows: &'a [Window], current: Option<u64>) -> Option<&'a Window> {
        let now = Instant::now();
        self.refused.retain(|(_, until)| now < *until);
        game_window::choose(&self.rules, windows, current)
            .filter(|window| !self.refused.iter().any(|(handle, _)| *handle == window.handle))
            // A minimised window gives no frames. An ffmpeg that fills in
            // for it may stay on it; any other would stall.
            .filter(|window| !window.minimised || (self.keeps_flowing && Some(window.handle) == current))
    }

    /// Leave `handle` alone for a while, longer each time: every try
    /// costs a moment of the buffer. Gives how long.
    fn refuse(&mut self, handle: u64) -> Duration {
        let times = match self.failures.iter_mut().find(|(known, _)| *known == handle) {
            Some((_, times)) => {
                *times += 1;
                *times
            }
            None => {
                // Handles are used again by other windows: an old count is not kept for ever.
                if self.failures.len() >= 64 {
                    self.failures.remove(0);
                }
                self.failures.push((handle, 1));
                1
            }
        };
        let wait = LEAVE_ALONE * 2u32.pow(times.min(7) - 1);
        self.refused.push((handle, Instant::now() + wait));
        wait
    }

    /// Whether `wanted` has been the one to record for long enough to
    /// change to it.
    fn settled(&mut self, wanted: Option<u64>, current: Option<u64>) -> bool {
        if wanted == current {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((pending, since)) if pending == wanted => {
                if since.elapsed() < SETTLE {
                    return false;
                }
                self.pending = None;
                true
            }
            _ => {
                self.pending = Some((wanted, Instant::now()));
                false
            }
        }
    }
}

/// A line for the log, and for the daemon, which reads from it how the
/// recording is made.
fn say_way(video: &Video, buffer_secs: u32) {
    note!("recording with {} at {} fps, {} kbit/s, a buffer of {} s", video.describe(), video.fps, video.bitrate_kbps, buffer_secs);
}

struct Recorder {
    ffmpeg: PathBuf,
    /// How the video is made now: `screen`, or the same of a game's window.
    video: Video,
    /// How the monitor is recorded, which is what there is when no game's
    /// window is.
    screen: Video,
    /// The size of every picture, whatever it is of.
    canvas: (u32, u32),
    /// The name of the monitor that is recorded.
    monitor: String,
    /// How the monitor was recorded when this recording began: the way to
    /// go back to when another had to be settled for.
    first: Video,
    /// When the way this began with is tried again, while another is used.
    better_at: Option<Instant>,
    /// How long after this try the next one is made.
    better_wait: Duration,
    /// When the room left on the ring's disk was last looked at.
    disk_looked: Instant,
    /// `None` when a game's window cannot be told or recorded here.
    follow: Option<Follow>,
    /// The game's window being recorded.
    target: Option<Window>,
    /// Nothing is recorded yet: the game's window is waited for, since then.
    waiting: Option<Instant>,
    /// What this ffmpeg can do, for choosing how to record over again.
    has: Abilities,
    /// The ways were tried over since ffmpeg last stayed up.
    chosen_again: bool,
    options: Options,
    out_dir: PathBuf,
    ring: PathBuf,
    sink: Arc<Mutex<Sink>>,
    leash: &'static ChildLeash,
    audio_offset_ms: i64,
    running: Option<Running>,
    /// When to start ffmpeg again, while it is not running.
    restart_at: Option<Instant>,
    short_runs: u32,
    /// The newest piece seen and when it appeared: ffmpeg's pulse.
    newest: Option<u64>,
    progressed: Instant,
}

impl Recorder {
    fn tracks(&self) -> usize {
        self.options.audio.len()
    }

    /// Take `video` as the way the monitor is recorded from now on, making
    /// pictures of `canvas` size. Pieces of another size or encoder would
    /// not join with what comes now: the ring starts over then, and a clip
    /// reaches back to this moment at the most.
    fn adopt(&mut self, video: Video, canvas: (u32, u32)) {
        let another_kind = canvas != self.canvas || video.encoder != self.screen.encoder;
        if another_kind {
            note!(
                "the picture is now {}x{} with {} (it was {}x{} with {}): the two do not join, so the buffer starts over and what was in it is lost",
                canvas.0,
                canvas.1,
                video.encoder.name,
                self.canvas.0,
                self.canvas.1,
                self.screen.encoder.name
            );
            ring::clear(&self.ring);
            if let Some(follow) = self.follow.as_mut() {
                follow.frames = None;
            }
        }
        self.canvas = canvas;
        self.screen = video;
        match &mut self.video.grab {
            // The game's window goes on being recorded, into the size the
            // pictures have now.
            Grab::Window { canvas: of_window, .. } if !another_kind => *of_window = canvas,
            Grab::Window { .. } => {
                // With another encoder: the window is found again, and its way tried over.
                self.video = self.screen.clone();
                self.target = None;
            }
            _ => self.video = self.screen.clone(),
        }
    }

    /// Note which way the monitor is recorded now: when it is not the one
    /// this recording began with, that one is tried again after a while.
    fn settle(&mut self) {
        if same_way(&self.screen, &self.first) {
            self.better_at = None;
        } else if self.better_at.is_none() {
            note!("{} is tried again in {} min", self.first.describe(), self.better_wait.as_secs() / 60);
            self.better_at = Some(Instant::now() + self.better_wait);
        }
    }

    /// Choose over again how the monitor is recorded.
    fn choose_again(&mut self) -> Result<(), String> {
        let chosen = choose_video(&self.ffmpeg, &self.has, &self.options)?;
        self.monitor = chosen.monitor;
        self.adopt(chosen.video, chosen.canvas);
        self.settle();
        Ok(())
    }

    /// Before the monitor is recorded again: look at it as it is now. A
    /// game may have given it another mode since the way to record it was
    /// chosen, and an ffmpeg started for the old one would make pictures
    /// of another size, or none.
    fn look_at_the_monitor(&mut self) -> Result<(), String> {
        if monitors::test_source() || self.target.is_some() {
            return Ok(());
        }
        let all = monitors::list();
        // Gone for now: the start fails, and after a few the ways are tried over.
        let Some(monitor) = monitors::pick(&self.monitor, &all) else { return Ok(()) };
        match screen_now(&self.screen, monitor, self.options.size) {
            Some((video, canvas)) if video == self.screen && canvas == self.canvas => {}
            Some((video, canvas)) => {
                note!("{} is now {}x{} at {},{}: recorded as it is now", monitor.name, monitor.size.0, monitor.size.1, monitor.origin.0, monitor.origin.1);
                self.adopt(video, canvas);
            }
            None => {
                note!("{} is now {}x{} and cannot be recorded as before: choosing how to record over again", monitor.name, monitor.size.0, monitor.size.1);
                self.choose_again()?;
                say_way(&self.video, self.options.buffer_secs);
            }
        }
        Ok(())
    }

    /// Try the way this recording began with, while another is used: for
    /// one second, beside the ffmpeg that is recording. When it works
    /// again, it is gone back to.
    fn try_better(&mut self) {
        if self.running.is_none() || !self.better_at.is_some_and(|at| Instant::now() >= at) {
            return;
        }
        // Whatever comes of it, the next try is further off: this cannot go round and round.
        self.better_wait = next_better_wait(self.better_wait);
        self.better_at = Some(Instant::now() + self.better_wait);
        let all = monitors::list();
        let now = monitors::pick(&self.monitor, &all).and_then(|monitor| screen_now(&self.first, monitor, self.options.size));
        let Some((video, canvas)) = now else {
            note!("{} does not fit the monitor as it is now; looked at again in {} min", self.first.describe(), self.better_wait.as_secs() / 60);
            return;
        };
        if let Err(err) = ffmpeg::run_quietly(&self.ffmpeg, &ffmpeg::probe_args(&video), ffmpeg::PROBE_LIMIT) {
            note!("{} still does not work ({err}); tried again in {} min", video.describe(), self.better_wait.as_secs() / 60);
            return;
        }
        note!("{} works again: going back to it from {}", video.describe(), self.screen.describe());
        self.adopt(video, canvas);
        // Should it not stay up after all, the ways are tried over once more.
        self.chosen_again = false;
        if self.target.is_none() {
            self.record(self.screen.clone(), None);
        }
        // Left as it is until the next try, should this one not last.
        self.better_at = None;
    }

    fn start(&mut self) -> Result<(), String> {
        self.look_at_the_monitor()?;
        let first = ring::next_start(&ring::indices(&self.ring));
        let args = ffmpeg::record_args(&self.video, self.tracks(), &self.ring, first, self.audio_offset_ms);
        note!("starting: {}", ffmpeg::shown(&self.ffmpeg, &args));
        let stdin = if self.tracks() > 0 { Stdio::piped() } else { Stdio::null() };
        let mut child = ffmpeg::recording_command(&self.ffmpeg)
            .args(&args)
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("ffmpeg ({}) could not be started: {err}", self.ffmpeg.display()))?;
        self.leash.hold(&child);
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            follow(stderr, tail.clone());
        }
        if let (Some(pipe), Ok(mut sink)) = (child.stdin.take(), self.sink.lock()) {
            sink.connect(pipe);
        }
        note!("ffmpeg is process {}, writing pieces from number {first}", child.id());
        self.running = Some(Running { child, started: Instant::now(), tail });
        self.progressed = Instant::now();
        Ok(())
    }

    /// The way to record `window`: as the monitor is, but of the window,
    /// its frames where the encoder can take them. Tried for a second the
    /// first time; `None` when no way works.
    fn window_video(&mut self, window: &Window) -> Option<Video> {
        let known = self.follow.as_ref()?.frames;
        let grab = Grab::Window { hwnd: window.handle, canvas: self.canvas };
        let video = |frames| Video { grab: grab.clone(), frames, scale: None, ..self.screen.clone() };
        if let Some(frames) = known {
            return Some(video(frames));
        }
        // The same encoder, or the pieces would not join.
        let tries: &[Frames] = if self.screen.encoder.family == Family::Software { &[Frames::Memory] } else { &[Frames::Gpu, Frames::Memory] };
        for frames in tries {
            let video = video(*frames);
            match ffmpeg::run_quietly(&self.ffmpeg, &ffmpeg::probe_args(&video), ffmpeg::PROBE_LIMIT) {
                Ok(()) => {
                    note!("{} works", video.describe());
                    self.follow.as_mut()?.frames = Some(*frames);
                    return Some(video);
                }
                Err(err) => note!("{} does not work here: {err}", video.describe()),
            }
        }
        None
    }

    fn leave_alone(&mut self, window: &Window) {
        if let Some(follow) = self.follow.as_mut() {
            let wait = follow.refuse(window.handle);
            note!("the window of {} is left alone for {} s; the monitor is recorded", window.program, wait.as_secs());
        }
        // The window that was waited for is there and cannot be recorded:
        // the monitor it is on can.
        if self.waiting.take().is_some() {
            self.record(self.screen.clone(), None);
        }
    }

    /// Look for the game's window, and change what is recorded when it has
    /// appeared, gone, or given way to another.
    fn follow_game(&mut self) {
        let Some(follow) = self.follow.as_mut() else { return };
        if follow.looked.elapsed() < WINDOWS_EVERY {
            return;
        }
        follow.looked = Instant::now();
        let current = self.target.as_ref().map(|window| window.handle);
        let windows = game_window::list();
        let wanted = follow.wanted(&windows, current).cloned();
        if !follow.settled(wanted.as_ref().map(|window| window.handle), current) {
            if wanted.is_none() && self.waiting.is_some_and(|since| since.elapsed() >= WINDOW_WAIT) {
                note!("no game's window after {} s: recording the monitor", WINDOW_WAIT.as_secs());
                self.waiting = None;
                self.record(self.screen.clone(), None);
            }
            return;
        }
        let video = match &wanted {
            None => {
                note!("the game's window is gone or minimised: recording the monitor");
                self.screen.clone()
            }
            Some(window) => {
                note!("a game's window: {:?} of {} (process {}), {}x{}", window.title, window.program, window.pid, window.size.0, window.size.1);
                let Some(video) = self.window_video(window) else {
                    self.leave_alone(window);
                    return;
                };
                video
            }
        };
        if self.waiting.take().is_some() {
            note!("the game's window is there: recording begins");
        }
        self.record(video, wanted);
    }

    /// Change what is recorded: a new ffmpeg, writing on into the ring.
    fn record(&mut self, video: Video, target: Option<Window>) {
        self.kill();
        self.video = video;
        self.target = target;
        say_way(&self.video, self.options.buffer_secs);
        // Now, or as soon as it can be: `look_after_ffmpeg` gives up when it cannot.
        self.restart_at = Some(Instant::now());
        self.short_runs = 0;
    }

    /// Stop ffmpeg now. Its pieces need no finishing, so it is simply killed.
    fn kill(&mut self) {
        if let Some(mut running) = self.running.take() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
        // Only now: the mixer may be inside a write that the kill ends.
        if let Ok(mut sink) = self.sink.lock() {
            sink.disconnect();
        }
    }

    /// Notice ffmpeg having ended or got stuck, and start it again when it
    /// is time. `Err` when it will not stay up.
    fn look_after_ffmpeg(&mut self) -> Result<(), String> {
        let indices = ring::indices(&self.ring);
        if indices.last().copied() != self.newest {
            self.newest = indices.last().copied();
            self.progressed = Instant::now();
        }
        for index in ring::expired(&indices, self.options.buffer_secs) {
            let _ = std::fs::remove_file(self.ring.join(ring::piece_name(*index, self.video.container())));
        }
        if self.disk_looked.elapsed() >= DISK_EVERY {
            self.disk_looked = Instant::now();
            // Said and ended here: left to itself a full disk is an ffmpeg
            // that ends as it starts, over and over, and says nothing clear.
            room_to_go_on(sys::free_bytes(&self.ring))?;
        }
        self.try_better();
        // Owed more sound than is worth writing as silence (the machine
        // slept): started afresh, it counts from now.
        let behind = self.sink.lock().is_ok_and(|sink| sink.fell_behind());
        let Some(running) = self.running.as_mut() else {
            if self.restart_at.is_some_and(|at| Instant::now() >= at) {
                self.restart_at = None;
                self.start()?;
            }
            return Ok(());
        };
        let ran = running.started.elapsed();
        // A window that shows nothing new gives no pieces, unless this
        // ffmpeg fills in for it: no reason to wait long for it.
        let fills_in = self.follow.as_ref().is_some_and(|follow| follow.keeps_flowing);
        let stall = if self.target.is_some() && !fills_in { WINDOW_STALL } else { STALL };
        let quiet = self.progressed.elapsed();
        let (ended, stalled, aged) = match running.child.try_wait() {
            Ok(Some(status)) => (Some(format!("ended by itself ({status})")), false, false),
            // Not before it has had the time to start that any ffmpeg gets.
            Ok(None) if quiet > STALL || (quiet > stall && ran > STALL) => (Some(format!("wrote nothing for {} s", quiet.as_secs())), true, false),
            Ok(None) if ran > MAX_RUN => (Some("has run for 20 hours and is started afresh".to_string()), false, true),
            Ok(None) if behind => (Some("is owed more sound than can be made up for and is started afresh".to_string()), false, true),
            Ok(None) => (None, false, false),
            Err(err) => (Some(format!("cannot be asked how it is ({err})")), false, false),
        };
        let Some(why) = ended else { return Ok(()) };
        let tail = running.tail.lock().map(|t| t.iter().cloned().collect::<Vec<_>>().join(" / ")).unwrap_or_default();
        self.kill();
        if let Some(window) = self.target.clone() {
            note!("ffmpeg {why} after {:.1} s of recording the game's window; its last words: {tail}", ran.as_secs_f64());
            if aged {
                // Nothing wrong with the window: the same again.
                self.restart_at = Some(Instant::now());
                return Ok(());
            }
            // A window that is closed ends its ffmpeg: the game is over,
            // and that is no failure. One still there that gave nothing,
            // or whose ffmpeg ended as it began, would not be recorded;
            // after a good while of recording it is simply looked for again.
            let there = game_window::list().iter().any(|other| other.handle == window.handle);
            if there && (stalled || ran < HEALTHY_AFTER) {
                self.leave_alone(&window);
            }
            self.record(self.screen.clone(), None);
            return Ok(());
        }
        self.short_runs = if ran >= HEALTHY_AFTER { 1 } else { self.short_runs + 1 };
        note!("ffmpeg {why} after {:.1} s; its last words: {tail}", ran.as_secs_f64());
        if self.short_runs > MAX_SHORT_RUNS {
            // The way that worked when this began may be gone: a monitor
            // plugged in again has another handle, and another place among
            // the outputs. Once, the ways are tried over.
            if std::mem::replace(&mut self.chosen_again, true) {
                return Err(format!("ffmpeg will not stay up: {tail}"));
            }
            note!("ffmpeg will not stay up: choosing how to record over again");
            self.choose_again()?;
            self.record(self.screen.clone(), None);
            return Ok(());
        }
        if ran >= HEALTHY_AFTER {
            self.chosen_again = false;
        }
        let wait = restart_delay(self.short_runs);
        note!("starting ffmpeg again in {} ms (try {})", wait.as_millis(), self.short_runs);
        self.restart_at = Some(Instant::now() + wait);
        Ok(())
    }

    /// The pieces of a clip saved now: wait for the piece being written to
    /// be finished, so the clip reaches up to this moment.
    fn pieces_for_clip(&mut self) -> Vec<u64> {
        let open = ring::indices(&self.ring).last().copied();
        let began = Instant::now();
        let alive = |r: &mut Recorder| r.running.as_mut().is_some_and(|run| matches!(run.child.try_wait(), Ok(None)));
        // The piece is finished when ffmpeg has begun the next one. A dead
        // ffmpeg begins nothing: what it left is all there will be.
        while alive(self) && began.elapsed() < PIECE_WAIT && ring::indices(&self.ring).last().copied() == open {
            std::thread::sleep(Duration::from_millis(40));
        }
        let indices = ring::indices(&self.ring);
        let Some(mut last) = indices.last().copied() else { return Vec::new() };
        if alive(self)
            && Some(last) != open
            && let Some(open) = open
        {
            // The one that was being written when the save was asked for.
            last = open;
        }
        let container = self.video.container();
        let filled = |index: &u64| std::fs::metadata(self.ring.join(ring::piece_name(*index, container))).is_ok_and(|m| m.len() > 0);
        ring::covering(&indices, last, self.options.buffer_secs).iter().copied().filter(filled).collect()
    }

    /// Write the buffer out as one MP4 and give its path.
    fn save(&mut self) -> Result<PathBuf, String> {
        let began = Instant::now();
        let pieces = self.pieces_for_clip();
        let (Some(first), Some(last)) = (pieces.first(), pieces.last()) else {
            return Err("nothing has been recorded yet".into());
        };
        let waited = began.elapsed();
        std::fs::create_dir_all(&self.out_dir).map_err(|err| format!("{} could not be made: {err}", self.out_dir.display()))?;
        let container = self.video.container();
        let size: u64 = pieces.iter().map(|index| std::fs::metadata(self.ring.join(ring::piece_name(*index, container))).map_or(0, |m| m.len())).sum();
        room_for_clip(size, sys::free_bytes(&self.out_dir))?;
        let list = self.ring.join(ring::LIST_NAME);
        std::fs::write(&list, ring::concat_list(&pieces, container)).map_err(|err| format!("the list of pieces could not be written: {err}"))?;
        let name = clip_name(chrono::Local::now().naive_local(), |name| self.out_dir.join(name).exists());
        let out = self.out_dir.join(&name);
        // Written under another name and renamed: nobody sees half a clip.
        let partial = self.out_dir.join(format!("{name}.part"));
        // A long buffer is a big clip, and is given the time to be written.
        let joined = ffmpeg::run_quietly(&self.ffmpeg, &ffmpeg::join_args(&list, &partial, self.video.encoder.codec), ffmpeg::join_limit(size));
        let _ = std::fs::remove_file(&list);
        if let Err(err) = joined {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("the pieces could not be joined: {err}"));
        }
        std::fs::rename(&partial, &out).map_err(|err| format!("{} could not be put in place: {err}", out.display()))?;
        let bytes = std::fs::metadata(&out).map_or(0, |m| m.len());
        note!(
            "saved {}: pieces {first}..={last} ({} of them, about {} s), {:.1} MB, in {} ms ({} ms waiting for the last piece)",
            out.display(),
            pieces.len(),
            pieces.len() as u32 * ring::PIECE_SECS,
            bytes as f64 / 1e6,
            began.elapsed().as_millis(),
            waited.as_millis()
        );
        Ok(out)
    }
}

/// Send each word on stdin down `words`; the end of stdin is a `Stop`.
fn listen(words: mpsc::Sender<Word>) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            match word(&line) {
                Some(word) => {
                    if words.send(word).is_err() {
                        return;
                    }
                }
                None if line.trim().is_empty() => {}
                None => note!("stdin: {:?} is not something this recorder knows", line.trim()),
            }
        }
        note!("stdin ended: whoever started this recorder is gone");
        let _ = words.send(Word::Stop);
    });
}

/// End for `what`. It is said again as the very last line: the last
/// thing a recorder said is what the daemon shows for why it ended. For
/// want of room on the disk it ends with a code of its own.
fn failed(what: &str) -> ExitCode {
    note!("giving up: {what}");
    if what.starts_with(NO_ROOM) { ExitCode::from(NO_ROOM_CODE) } else { ExitCode::FAILURE }
}

/// Record until told to stop.
pub fn run(options: Options) -> ExitCode {
    crate::log::init(&crate::sys::logs_dir());
    note!("zenith-rec {} on {}", env!("CARGO_PKG_VERSION"), crate::sys::os_description());
    note!("asked for: {options:?}");
    if options.hdr_asked {
        note!("HDR or 10 bits were asked for and are not recorded: the clips are plain 8-bit {}", options.codec.as_str());
    }
    let ring_dir = sys::ring_dir();
    // Before anything is started: a ring that cannot fit is not begun.
    let ring_size = ring_bytes(options.bitrate_kbps, options.audio.len(), options.buffer_secs);
    if let Err(err) = room_to_begin(ring_size, sys::free_bytes(&ring_dir)) {
        return failed(&format!("{err} (the buffer is kept in {})", ring_dir.display()));
    }
    let ffmpeg = ffmpeg::locate();
    let has = ffmpeg::abilities(&ffmpeg);
    if has.version.is_empty() {
        return failed(&format!("ffmpeg ({}) cannot be run", ffmpeg.display()));
    }
    note!("{} at {}", has.version, ffmpeg.display());
    let out_dir = std::path::absolute(&options.out_dir).unwrap_or_else(|_| options.out_dir.clone());
    // What a recorder that was ended while it saved left half written.
    for left in stale_partials(&out_dir) {
        note!("clearing away a clip left half written: {}", left.display());
        let _ = std::fs::remove_file(left);
    }
    let Chosen { video, canvas, monitor } = match choose_video(&ffmpeg, &has, &options) {
        Ok(chosen) => chosen,
        Err(err) => return failed(&err),
    };
    say_way(&video, options.buffer_secs);
    let follow = match Follow::possible(&options, &has, std::env::var("ZENITH_REC_CAPTURE").ok().as_deref()) {
        Ok(rules) => {
            let keeps_flowing = ffmpeg::capture_keeps_flowing(&ffmpeg);
            note!("a game's window is recorded in place of the monitor while there is one (kept while minimised: {keeps_flowing}); games are {rules:?}");
            Some(Follow { rules, frames: None, keeps_flowing, looked: Instant::now(), pending: None, refused: Vec::new(), failures: Vec::new() })
        }
        Err(why) => {
            note!("the monitor is recorded, game or not: {why}");
            None
        }
    };
    if let Err(err) = std::fs::create_dir_all(&ring_dir) {
        return failed(&format!("the ring directory {} could not be made: {err}", ring_dir.display()));
    }
    ring::clear(&ring_dir);
    if cfg!(windows) {
        // What a recorder that was ended without a word left behind.
        for left in ring::forgotten(&crate::sys::rings_dir(), &ring_dir, std::time::SystemTime::now()) {
            note!("clearing away a ring left behind: {}", left.display());
            let _ = std::fs::remove_dir_all(left);
        }
    }
    note!("ring in {}, about {} MB when it is full", ring_dir.display(), megabytes(ring_size));

    let stop = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(Mutex::new(Sink::for_tracks(options.audio.len())));
    let mut threads = Vec::new();
    if !options.audio.is_empty() {
        let table = Arc::new(ProcessTable::default());
        if options.audio.iter().any(|value| crate::audio::parse_spec(value).needs_processes()) {
            // Filled once before the tracks plan by it.
            table.replace(processes::Scanner::new().scan());
            let (table, stop) = (table.clone(), stop.clone());
            threads.push(std::thread::spawn(move || processes::keep_scanning(table, stop)));
        }
        let (values, sink, stop) = (options.audio.clone(), sink.clone(), stop.clone());
        threads.push(std::thread::spawn(move || mixer::run(|| crate::audio::sources(&values, &table), sink, stop)));
    }

    // The setting, unless the environment says otherwise (for trying one out).
    let audio_offset_ms = std::env::var("ZENITH_REC_AUDIO_OFFSET_MS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(options.audio_offset_ms);
    let mut recorder = Recorder {
        ffmpeg,
        screen: video.clone(),
        first: video.clone(),
        video,
        canvas,
        monitor,
        better_at: None,
        better_wait: BETTER_AFTER,
        disk_looked: Instant::now(),
        // Only where a game's window can be told at all.
        waiting: (options.wait_for_game && follow.is_some()).then(Instant::now),
        follow,
        target: None,
        has,
        chosen_again: false,
        options,
        out_dir,
        ring: ring_dir.clone(),
        sink,
        leash: crate::sys::leash(),
        audio_offset_ms,
        running: None,
        restart_at: None,
        short_runs: 0,
        newest: None,
        progressed: Instant::now(),
    };
    let (words_in, words) = mpsc::channel();
    listen(words_in);
    let mut code = ExitCode::SUCCESS;
    let mut gave_up = None;
    if recorder.waiting.is_some() {
        note!("nothing is recorded until the game's window is on screen (for {} s at most)", WINDOW_WAIT.as_secs());
    }
    // Waiting, there is no ffmpeg yet: `follow_game` starts it.
    if let Err(err) = if recorder.waiting.is_some() { Ok(()) } else { recorder.start() } {
        code = failed(&err);
        gave_up = Some(err);
    } else {
        loop {
            match words.recv_timeout(LOOK_EVERY) {
                Ok(Word::Save) => {
                    // The one thing ever written to stdout is the answer to
                    // a `save`: the clip's path, which the daemon reads as
                    // the clip, or why there is none.
                    let answer = match recorder.save() {
                        Ok(path) => path.display().to_string(),
                        Err(err) => {
                            note!("save failed: {err}");
                            save_failed_line(&err)
                        }
                    };
                    let mut stdout = std::io::stdout().lock();
                    let _ = writeln!(stdout, "{answer}");
                    let _ = stdout.flush();
                }
                Ok(Word::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            recorder.follow_game();
            if let Err(err) = recorder.look_after_ffmpeg() {
                code = failed(&err);
                gave_up = Some(err);
                break;
            }
        }
    }
    note!("stopping");
    recorder.kill();
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        let _ = thread.join();
    }
    ring::clear(&ring_dir);
    // The directory was made for this run alone.
    let _ = std::fs::remove_dir(&ring_dir);
    match gave_up {
        // The last line is the one the daemon shows for why it ended.
        Some(why) => note!("stopped: {why}"),
        None => note!("stopped"),
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_knows_two_words() {
        assert_eq!(word("save"), Some(Word::Save));
        assert_eq!(word("  save\r"), Some(Word::Save));
        assert_eq!(word("stop"), Some(Word::Stop));
        assert_eq!(word("quit"), Some(Word::Stop));
        assert_eq!(word(""), None);
        assert_eq!(word("SAVE"), None);
        assert_eq!(word("save now"), None);
    }

    #[test]
    fn a_clip_is_named_for_its_moment_and_never_overwrites() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap().and_hms_opt(18, 4, 9).unwrap();
        assert_eq!(clip_name(now, |_| false), "Replay_2026-11-20_18-04-09.mp4");
        let taken = ["Replay_2026-11-20_18-04-09.mp4", "Replay_2026-11-20_18-04-09_2.mp4"];
        assert_eq!(clip_name(now, |name| taken.contains(&name)), "Replay_2026-11-20_18-04-09_3.mp4");
    }

    #[test]
    fn restarts_wait_longer_each_time_up_to_a_point() {
        assert_eq!(restart_delay(0), Duration::from_millis(500));
        assert_eq!(restart_delay(1), Duration::from_millis(500));
        assert!(restart_delay(3) > restart_delay(2));
        assert_eq!(restart_delay(50), restart_delay(8));
    }

    #[test]
    fn a_save_that_failed_is_answered_with_a_line_that_is_no_path() {
        let line = save_failed_line("the pieces could not be joined: exit status: 1\nmore");
        assert_eq!(line, "save-failed: the pieces could not be joined: exit status: 1 more");
        assert!(!Path::new(&line).is_absolute(), "a reader that only knows paths passes over it");
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn only_clips_left_half_written_are_cleared_away() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["Replay_2026-11-20_18-04-09.mp4", "Replay_2026-11-20_18-04-09.mp4.part", "Replay_2026-11-20_18-04-09_2.mp4.part", "holiday.mp4.part", "Replay_notes.part"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let found: Vec<String> = stale_partials(dir.path()).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(found, ["Replay_2026-11-20_18-04-09.mp4.part", "Replay_2026-11-20_18-04-09_2.mp4.part"]);
        assert!(stale_partials(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn a_ring_is_begun_only_where_it_fits() {
        // A minute at 20 Mbit/s with three tracks: about 190 MB.
        let ring = ring_bytes(20_000, 3, 60);
        assert!((180_000_000..200_000_000).contains(&ring), "{ring}");
        assert_eq!(room_to_begin(ring, None), Ok(()), "where it cannot be told, it is tried");
        assert_eq!(room_to_begin(ring, Some(ring + ROOM_TO_SPARE)), Ok(()));
        let err = room_to_begin(ring, Some(ring)).unwrap_err();
        assert!(err.starts_with(NO_ROOM) && err.contains("MB are free"), "{err}");
        // A day of buffer is more than was meant, however big the disk.
        let day = ring_bytes(20_000, 3, 86_400);
        let err = room_to_begin(day, Some(u64::MAX)).unwrap_err();
        assert!(err.starts_with(NO_ROOM) && err.contains("the most one may"), "{err}");
        // The longest buffer the app offers, at a bitrate above any it sets.
        assert_eq!(room_to_begin(ring_bytes(150_000, 3, 600), Some(u64::MAX)), Ok(()));
    }

    #[test]
    fn a_disk_that_fills_up_ends_the_recording_with_its_own_reason() {
        assert_eq!(room_to_go_on(None), Ok(()));
        assert_eq!(room_to_go_on(Some(LEAST_ROOM)), Ok(()));
        let err = room_to_go_on(Some(LEAST_ROOM - 1)).unwrap_err();
        assert!(err.starts_with(NO_ROOM), "{err}");
        // A clip that does not fit where the clips go is that save's failure.
        assert_eq!(room_for_clip(100 << 20, Some(200 << 20)), Ok(()));
        assert_eq!(room_for_clip(100 << 20, None), Ok(()));
        assert!(room_for_clip(100 << 20, Some(100 << 20)).unwrap_err().starts_with(NO_ROOM));
    }

    fn screen(grab: Grab, frames: Frames, family: Family) -> Video {
        let encoder = ffmpeg::encoders(Codec::H264).into_iter().find(|e| e.family == family).unwrap();
        Video { grab, frames, encoder, fps: 60, cursor: true, scale: None, bitrate_kbps: 20_000 }
    }

    fn monitor(size: (u32, u32), output_idx: Option<u32>) -> Monitor {
        Monitor { name: "DISPLAY1".into(), size, origin: (0, 0), primary: true, output_idx, handle: 65537 }
    }

    #[test]
    fn a_monitor_given_another_mode_is_recorded_at_the_size_it_has_now() {
        let was = screen(Grab::Ddagrab { output_idx: 0 }, Frames::Gpu, Family::Nvenc);
        // Nothing changed: the same way, the same size.
        assert_eq!(screen_now(&was, &monitor((2560, 1440), Some(0)), None), Some((was.clone(), (2560, 1440))));
        // 1440p became 1080p: the pictures are 1080p from here on, which
        // is what makes the ring start over.
        let (now, canvas) = screen_now(&was, &monitor((1920, 1080), Some(0)), None).unwrap();
        assert_eq!((now, canvas), (was.clone(), (1920, 1080)));
        // The copied pixels are asked for at the size and place it has now.
        let pixels = screen(Grab::Gdigrab { origin: (0, 0), size: (2560, 1440) }, Frames::Memory, Family::Software);
        let moved = Monitor { origin: (-1920, 0), ..monitor((1920, 1080), None) };
        let (now, canvas) = screen_now(&pixels, &moved, None).unwrap();
        assert_eq!(now.grab, Grab::Gdigrab { origin: (-1920, 0), size: (1920, 1080) });
        assert_eq!(canvas, (1920, 1080));
        // Plugged in again, it has another handle and another place among the outputs.
        let again = Monitor { handle: 131073, ..monitor((2560, 1440), Some(1)) };
        assert_eq!(screen_now(&was, &again, None).unwrap().0.grab, Grab::Ddagrab { output_idx: 1 });
        let graphics = screen(Grab::Gfxcapture { hmonitor: 65537 }, Frames::Gpu, Family::Amf);
        assert_eq!(screen_now(&graphics, &again, None).unwrap().0.grab, Grab::Gfxcapture { hmonitor: 131073 });
    }

    #[test]
    fn a_size_that_was_asked_for_is_kept_whatever_the_monitors_mode() {
        // `-s 1920x1080` on a 1440p monitor: scaled, and still 1080p when
        // the monitor itself becomes 1080p, with nothing left to scale.
        let mut was = screen(Grab::Gfxcapture { hmonitor: 65537 }, Frames::Gpu, Family::Nvenc);
        was.scale = Some((1920, 1080));
        let (now, canvas) = screen_now(&was, &monitor((1920, 1080), Some(0)), Some((1920, 1080))).unwrap();
        assert_eq!((now.scale, canvas), (None, (1920, 1080)), "the same size: the ring is kept");
        let (back, canvas) = screen_now(&now, &monitor((2560, 1440), Some(0)), Some((1920, 1080))).unwrap();
        assert_eq!((back.scale, canvas), (Some((1920, 1080)), (1920, 1080)));
        // Desktop Duplication kept on the card cannot be scaled: another
        // way has to be chosen, as it has when the monitor left the first card.
        let dda = screen(Grab::Ddagrab { output_idx: 0 }, Frames::Gpu, Family::Nvenc);
        assert_eq!(screen_now(&dda, &monitor((2560, 1440), Some(0)), Some((1920, 1080))), None);
        assert_eq!(screen_now(&dda, &monitor((1920, 1080), None), None), None);
        let downloaded = Video { frames: Frames::Memory, ..dda };
        assert_eq!(screen_now(&downloaded, &monitor((2560, 1440), Some(0)), Some((1920, 1080))).unwrap().0.scale, Some((1920, 1080)));
    }

    #[test]
    fn the_way_it_began_with_is_tried_again_ever_less_often() {
        let mut wait = BETTER_AFTER;
        let mut waits = Vec::new();
        for _ in 0..8 {
            waits.push(wait.as_secs() / 60);
            wait = next_better_wait(wait);
        }
        assert_eq!(waits, [5, 10, 20, 40, 60, 60, 60, 60]);
        // Which way it is, is told without the monitor's place or size.
        let dda = screen(Grab::Ddagrab { output_idx: 0 }, Frames::Gpu, Family::Nvenc);
        assert!(same_way(&dda, &Video { grab: Grab::Ddagrab { output_idx: 2 }, scale: Some((1280, 720)), ..dda.clone() }));
        assert!(!same_way(&dda, &Video { frames: Frames::Memory, ..dda.clone() }));
        assert!(!same_way(&dda, &screen(Grab::Gdigrab { origin: (0, 0), size: (1920, 1080) }, Frames::Gpu, Family::Nvenc)));
        assert!(!same_way(&dda, &screen(Grab::Ddagrab { output_idx: 0 }, Frames::Gpu, Family::Software)));
    }

    fn follow(keeps_flowing: bool) -> Follow {
        let rules = Rules::new(&[r"C:\Steam\steamapps\common"], &[]);
        Follow { rules, frames: None, keeps_flowing, looked: Instant::now(), pending: None, refused: Vec::new(), failures: Vec::new() }
    }

    fn game(handle: u64) -> Window {
        let program = r"C:\Steam\steamapps\common\Game\game.exe".to_string();
        Window { handle, pid: 7, program, title: "Game".into(), size: (1920, 1080), minimised: false, front: true }
    }

    #[test]
    fn a_window_is_changed_to_only_once_it_has_stayed_the_one() {
        let mut follow = follow(false);
        assert!(!follow.settled(None, None), "nothing to change");
        assert!(!follow.settled(Some(5), None), "just appeared");
        assert!(!follow.settled(Some(5), None), "not for long enough");
        // Another takes its place: the wait starts over.
        assert!(!follow.settled(Some(6), None));
        follow.pending = Some((Some(6), Instant::now() - SETTLE));
        assert!(follow.settled(Some(6), None));
        assert_eq!(follow.pending, None);
        // Gone again before it settled: nothing is changed.
        assert!(!follow.settled(Some(5), None));
        assert!(!follow.settled(None, None));
        assert_eq!(follow.pending, None);
    }

    #[test]
    fn a_minimised_window_is_kept_only_by_an_ffmpeg_that_fills_in_for_it() {
        let minimised = [Window { minimised: true, size: (0, 0), front: false, ..game(5) }];
        assert_eq!(follow(false).wanted(&minimised, Some(5)), None, "it would stall: the monitor is recorded meanwhile");
        assert_eq!(follow(true).wanted(&minimised, Some(5)).map(|w| w.handle), Some(5));
        assert_eq!(follow(true).wanted(&minimised, None), None, "but none is started on one");
        assert_eq!(follow(false).wanted(&[game(5)], None).map(|w| w.handle), Some(5));
    }

    #[test]
    fn a_window_that_would_not_be_recorded_is_left_alone_for_a_while() {
        let mut follow = follow(false);
        follow.refused.push((5, Instant::now() + LEAVE_ALONE));
        assert_eq!(follow.wanted(&[game(5)], None), None);
        assert_eq!(follow.wanted(&[game(5), Window { front: false, ..game(6) }], None).map(|w| w.handle), None, "the one in front is the game's");
        follow.refused = vec![(5, Instant::now())];
        assert_eq!(follow.wanted(&[game(5)], None).map(|w| w.handle), Some(5), "and tried again after");
    }

    #[test]
    fn a_window_that_keeps_failing_is_left_alone_longer_each_time() {
        let mut follow = follow(false);
        assert_eq!(follow.refuse(5), LEAVE_ALONE);
        assert_eq!(follow.refuse(5), LEAVE_ALONE * 2);
        assert_eq!(follow.refuse(6), LEAVE_ALONE, "each window by itself");
        for _ in 0..20 {
            follow.refuse(5);
        }
        assert_eq!(follow.refuse(5), LEAVE_ALONE * 64, "and not without end");
        for handle in 100..200 {
            follow.refuse(handle);
        }
        assert!(follow.failures.len() <= 64);
    }

    #[test]
    fn a_window_is_followed_only_where_it_can_be() {
        let crate::args::Command::Record(mut options) = crate::args::parse(&["-o".into(), "/c".into()]).unwrap() else { panic!() };
        let mut has = Abilities::default();
        assert!(Follow::possible(&options, &has, None).unwrap_err().contains("which programs are games"));
        options.game_programs.push("star.exe".into());
        if monitors::test_source() {
            assert!(Follow::possible(&options, &has, None).unwrap_err().contains("test picture"));
        } else {
            assert!(Follow::possible(&options, &has, None).unwrap_err().contains("gfxcapture"));
            has.filters.insert("gfxcapture".into());
            assert!(Follow::possible(&options, &has, Some("ddagrab")).is_err());
            assert!(Follow::possible(&options, &has, Some("gfxcapture")).is_ok());
            assert!(Follow::possible(&options, &has, None).is_ok());
        }
    }

    #[test]
    fn a_test_recording_is_software_at_the_asked_size() {
        let crate::args::Command::Record(mut options) = crate::args::parse(&["-o".into(), "/c".into(), "-k".into(), "av1".into()]).unwrap() else { panic!() };
        let mut has = Abilities::default();
        has.encoders.insert("libx264".into());
        // AV1 asked for, but this ffmpeg has no software AV1.
        let video = test_video(&options, &has);
        assert_eq!(video.grab, Grab::Test { size: (1280, 720) });
        assert_eq!((video.encoder.name, video.encoder.codec), ("libx264", Codec::H264));
        assert_eq!(video.container(), ring::Container::MpegTs);
        has.encoders.insert("libsvtav1".into());
        options.size = Some((320, 180));
        let video = test_video(&options, &has);
        assert_eq!(video.grab, Grab::Test { size: (320, 180) });
        assert_eq!(video.encoder.name, "libsvtav1");
        assert_eq!(video.container(), ring::Container::FragmentedMp4);
    }
}
