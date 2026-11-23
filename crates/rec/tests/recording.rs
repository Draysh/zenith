//! The whole recorder, for real: the built `zenith-rec` with ffmpeg's test
//! picture in place of a screen and test tones in place of sound devices.
//! Skipped, with a note, where there is no ffmpeg; unless
//! `ZENITH_REQUIRE_FFMPEG=1` says the tests are run to prove something,
//! where a skip would pass for a proof: then they fail.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn have(program: &str) -> bool {
    Command::new(program).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn ffmpeg_missing(test: &str) -> bool {
    let missing = !have("ffmpeg") || !have("ffprobe");
    if missing {
        assert!(std::env::var("ZENITH_REQUIRE_FFMPEG").as_deref() != Ok("1"), "{test}: ffmpeg and ffprobe are not installed, and ZENITH_REQUIRE_FFMPEG=1 says they have to be");
        println!("NOTE: {test} skipped: ffmpeg and ffprobe are not installed");
    }
    missing
}

/// A running recorder and everything it says.
struct Recorder {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    stderr: Arc<Mutex<Vec<String>>>,
    dir: tempfile::TempDir,
}

impl Recorder {
    fn start(buffer_secs: u32, tracks: &[&str]) -> Self {
        Self::start_with("h264", buffer_secs, tracks)
    }

    fn start_with(codec: &str, buffer_secs: u32, tracks: &[&str]) -> Self {
        Self::start_in(tempfile::tempdir().unwrap(), codec, buffer_secs, tracks, &[])
    }

    /// Started with `dir` as it was made ready, and `env` set besides.
    fn start_in(dir: tempfile::TempDir, codec: &str, buffer_secs: u32, tracks: &[&str], env: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_zenith-rec"));
        command.args(["-w", "screen", "-c", "mp4", "-f", "30", "-s", "320x180", "-k", codec, "-bm", "cbr", "-q", "1000"]);
        command.args(["-r", &buffer_secs.to_string(), "-replay-storage", "ram", "-keyint", "1", "-ac", "aac", "-cursor", "yes"]);
        for track in tracks {
            command.args(["-a", track]);
        }
        command.args(["-v", "no", "-o"]).arg(dir.path().join("clips"));
        command.env("ZENITH_REC_TEST_SOURCE", "1").env("ZENITH_REC_TEST_TONE", "1").env("ZENITH_REC_LOG_DIR", dir.path().join("logs"));
        command.envs(env.iter().copied());
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let (tx, stdout) = mpsc::channel();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let (err, lines) = (child.stderr.take().unwrap(), stderr.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                lines.lock().unwrap().push(line);
            }
        });
        let stdin = child.stdin.take();
        Self { child, stdin, stdout, stderr, dir }
    }

    fn say(&mut self, word: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{word}").unwrap();
        stdin.flush().unwrap();
    }

    fn log(&self) -> String {
        self.stderr.lock().unwrap().join("\n")
    }

    /// The ffmpeg processes it has started so far.
    fn ffmpeg_pids(&self) -> Vec<u32> {
        let marker = "ffmpeg is process ";
        let lines = self.stderr.lock().unwrap();
        lines.iter().filter_map(|l| l.split_once(marker)?.1.split(',').next()?.trim().parse().ok()).collect()
    }

    fn ring_dir(&self) -> Option<PathBuf> {
        let lines = self.stderr.lock().unwrap();
        lines.iter().find_map(|l| l.split_once(" ring in ").map(|(_, dir)| PathBuf::from(dir.trim())))
    }

    /// Wait until `ready` holds of what it has logged.
    fn wait_for(&self, what: &str, ready: impl Fn(&Self) -> bool) {
        let began = Instant::now();
        while !ready(self) {
            assert!(began.elapsed() < Duration::from_secs(20), "waited too long for {what}:\n{}", self.log());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Ask for a clip and read its path from stdout.
    fn save(&mut self) -> PathBuf {
        self.say("save");
        match self.stdout.recv_timeout(Duration::from_secs(20)) {
            Ok(line) => PathBuf::from(line),
            Err(_) => panic!("no clip path on stdout:\n{}", self.log()),
        }
    }

    /// Wait for it to exit; its exit code.
    fn finish(&mut self) -> Option<i32> {
        let began = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            if began.elapsed() > Duration::from_secs(20) {
                let _ = self.child.kill();
                panic!("it did not stop:\n{}", self.log());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// After it has exited: no ffmpeg of its own is left, nor its ring, and
    /// nothing but clip paths was ever on stdout.
    fn assert_clean(&self) {
        for pid in self.ffmpeg_pids() {
            assert!(!Path::new(&format!("/proc/{pid}")).exists(), "ffmpeg {pid} was left behind:\n{}", self.log());
        }
        if let Some(ring) = self.ring_dir() {
            assert!(!ring.exists(), "the ring {} was left behind", ring.display());
        }
        assert!(self.stdout.try_recv().is_err(), "something more was printed on stdout");
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // A failed test must not leave a recorder, or its ffmpeg, running.
        let _ = self.child.kill();
        let _ = self.child.wait();
        for pid in self.ffmpeg_pids() {
            if Path::new(&format!("/proc/{pid}")).exists() {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
        }
        if let Some(ring) = self.ring_dir().filter(|ring| ring.file_name().is_some_and(|n| n.to_string_lossy().starts_with("zenith-rec-ring-"))) {
            let _ = std::fs::remove_dir_all(ring);
        }
    }
}

fn has_encoder(name: &str) -> bool {
    let out = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some(name))
}

/// The codec of the clip's picture.
fn video_codec(clip: &Path) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name", "-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(clip)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// How the clip says its colours are to be read: range, matrix, transfer, primaries.
fn colours(clip: &Path) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=color_range,color_space,color_transfer,color_primaries", "-of", "csv=p=0"])
        .arg(clip)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The kinds of the clip's streams in order, and its length in seconds.
fn probe(clip: &Path) -> (Vec<String>, f64) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type:format=duration", "-of", "default=noprint_wrappers=1"])
        .arg(clip)
        .output()
        .unwrap();
    assert!(out.status.success(), "ffprobe: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let kinds = text.lines().filter_map(|l| l.strip_prefix("codec_type=")).map(String::from).collect();
    let duration = text.lines().find_map(|l| l.strip_prefix("duration=")).and_then(|d| d.parse().ok()).unwrap_or(0.0);
    (kinds, duration)
}

/// The pitch of the clip's `track`th sound stream, from how often it
/// crosses zero.
fn pitch(clip: &Path, track: usize) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(clip)
        .args(["-map", &format!("0:a:{track}"), "-ac", "1", "-ar", "48000", "-f", "s16le", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    let samples: Vec<i16> = out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    // The middle, clear of the ends.
    let middle = &samples[samples.len() / 4..samples.len() * 3 / 4];
    let crossings = middle.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    crossings as f64 / 2.0 / (middle.len() as f64 / 48000.0)
}

#[test]
fn a_clip_is_saved_with_its_tracks_in_order_and_stop_ends_it() {
    if ffmpeg_missing("saving a clip") {
        return;
    }
    let mut rec = Recorder::start(3, &["default_output", "default_input"]);
    std::thread::sleep(Duration::from_secs(5));
    let clip = rec.save();
    assert!(clip.is_absolute(), "{}", clip.display());
    assert_eq!(clip.parent(), Some(rec.dir.path().join("clips").as_path()));
    let name = clip.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("Replay_") && name.ends_with(".mp4"), "{name}");
    assert!(clip.is_file(), "{} is not there:\n{}", clip.display(), rec.log());

    let (kinds, duration) = probe(&clip);
    assert_eq!(kinds, ["video", "audio", "audio"], "the picture first, then a stream a track");
    assert!((2.5..=3.7).contains(&duration), "a 3 s buffer gave a clip of {duration} s:\n{}", rec.log());
    // The test tones are 440 Hz on the first track and 880 Hz on the second.
    let (first, second) = (pitch(&clip, 0), pitch(&clip, 1));
    assert!((425.0..455.0).contains(&first), "first track at {first} Hz");
    assert!((850.0..910.0).contains(&second), "second track at {second} Hz");
    // No player has to guess how the colours are meant.
    assert_eq!(colours(&clip), "tv,bt709,bt709,bt709", "{}", rec.log());

    // A word it does not know changes nothing; a second clip gets its own name.
    rec.say("hello");
    let again = rec.save();
    assert_ne!(again, clip);
    assert_eq!(probe(&again).0, ["video", "audio", "audio"]);
    assert!(!rec.dir.path().join("clips").read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")));

    rec.say("stop");
    assert_eq!(rec.finish(), Some(0), "{}", rec.log());
    rec.assert_clean();
    assert!(rec.dir.path().join("logs/zenith-rec.log").is_file(), "the log is written to a file as well");
}

#[test]
fn a_save_that_fails_is_answered_on_stdout_and_the_recording_goes_on() {
    if ffmpeg_missing("answering a save that fails") {
        return;
    }
    // Where the clips should go there is a file: no folder can be made.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("clips"), b"in the way").unwrap();
    let mut rec = Recorder::start_in(dir, "h264", 3, &["default_output"], &[]);
    std::thread::sleep(Duration::from_millis(2500));
    let answer = rec.save();
    let answer = answer.to_string_lossy();
    assert!(answer.starts_with("save-failed: ") && answer.contains("could not be made"), "{answer}\n{}", rec.log());
    assert!(!Path::new(&*answer).is_absolute(), "never mistaken for a clip's path");
    assert!(rec.log().contains("save failed: "));
    // Not ended by it: once there is somewhere to put it, the clip is saved.
    std::fs::remove_file(rec.dir.path().join("clips")).unwrap();
    let clip = rec.save();
    assert!(clip.is_file(), "{}:\n{}", clip.display(), rec.log());
    rec.say("stop");
    assert_eq!(rec.finish(), Some(0), "{}", rec.log());
    rec.assert_clean();
}

#[test]
fn a_clip_left_half_written_by_a_recorder_that_was_killed_is_cleared_away() {
    if ffmpeg_missing("clearing half-written clips away") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let clips = dir.path().join("clips");
    std::fs::create_dir_all(&clips).unwrap();
    std::fs::write(clips.join("Replay_2026-11-20_18-04-09.mp4.part"), b"half").unwrap();
    std::fs::write(clips.join("Replay_2026-11-20_18-04-09.mp4"), b"whole").unwrap();
    let mut rec = Recorder::start_in(dir, "h264", 3, &[], &[]);
    rec.wait_for("ffmpeg to start", |r| !r.ffmpeg_pids().is_empty());
    assert!(!clips.join("Replay_2026-11-20_18-04-09.mp4.part").exists(), "{}", rec.log());
    assert!(clips.join("Replay_2026-11-20_18-04-09.mp4").exists(), "a finished clip is never touched");
    rec.say("stop");
    assert_eq!(rec.finish(), Some(0), "{}", rec.log());
    rec.assert_clean();
}

#[test]
fn a_disk_with_no_room_for_the_buffer_ends_it_with_a_reason_and_code_of_its_own() {
    // A megabyte free, the test says: no ffmpeg is needed to see that a
    // buffer does not fit.
    let mut rec = Recorder::start_in(tempfile::tempdir().unwrap(), "h264", 30, &["default_output"], &[("ZENITH_REC_TEST_FREE_BYTES", "1000000")]);
    assert_eq!(rec.finish(), Some(3), "{}", rec.log());
    let log = rec.log();
    let last = log.lines().last().unwrap_or_default();
    assert!(last.contains("giving up: no room on the disk for the replay buffer") && last.contains("1 MB are free"), "the last line is the one the daemon shows: {last}");
    assert!(rec.ffmpeg_pids().is_empty(), "nothing was started");
    rec.assert_clean();
}

#[test]
fn the_end_of_stdin_stops_it() {
    if ffmpeg_missing("stopping when stdin ends") {
        return;
    }
    let mut rec = Recorder::start(5, &["default_output"]);
    rec.wait_for("ffmpeg to start", |r| !r.ffmpeg_pids().is_empty());
    std::thread::sleep(Duration::from_millis(1500));
    // The daemon dies: its end of the pipe closes.
    rec.stdin = None;
    assert_eq!(rec.finish(), Some(0), "{}", rec.log());
    rec.assert_clean();
}

// Killing by process number is done with `kill`, which Windows lacks.
#[cfg(unix)]
#[test]
fn the_buffer_survives_ffmpeg_dying() {
    if ffmpeg_missing("restarting ffmpeg") {
        return;
    }
    let mut rec = Recorder::start(10, &["default_output", "default_input"]);
    rec.wait_for("ffmpeg to start", |r| !r.ffmpeg_pids().is_empty());
    std::thread::sleep(Duration::from_millis(3500));
    let first = rec.ffmpeg_pids()[0];
    assert!(Command::new("kill").args(["-9", &first.to_string()]).status().unwrap().success());
    rec.wait_for("ffmpeg to be started again", |r| r.ffmpeg_pids().len() == 2);
    std::thread::sleep(Duration::from_millis(3500));

    let clip = rec.save();
    let (kinds, duration) = probe(&clip);
    assert_eq!(kinds, ["video", "audio", "audio"]);
    // About three seconds from before the kill and three from after.
    assert!((4.5..=8.5).contains(&duration), "{duration} s:\n{}", rec.log());
    assert!(rec.log().contains("writing pieces from number 0"));
    assert!(!rec.log().lines().filter(|l| l.contains("writing pieces from number")).nth(1).unwrap().ends_with("number 0"), "the numbering goes on");

    rec.say("stop");
    assert_eq!(rec.finish(), Some(0), "{}", rec.log());
    rec.assert_clean();
}

#[test]
fn a_recording_with_no_sound_tracks_is_only_the_picture() {
    if ffmpeg_missing("recording without sound") {
        return;
    }
    let mut rec = Recorder::start(3, &[]);
    std::thread::sleep(Duration::from_millis(3500));
    let clip = rec.save();
    assert_eq!(probe(&clip).0, ["video"]);
    rec.say("stop");
    assert_eq!(rec.finish(), Some(0), "{}", rec.log());
    rec.assert_clean();
}

/// The other codecs go through the same ring; AV1 in pieces of another
/// kind, since MPEG-TS cannot hold it.
#[test]
fn the_other_codecs_are_saved_too() {
    if ffmpeg_missing("saving H.265 and AV1 clips") {
        return;
    }
    for (codec, encoder) in [("hevc", "libx265"), ("av1", "libsvtav1")] {
        if !has_encoder(encoder) {
            println!("NOTE: saving a {codec} clip skipped: this ffmpeg has no {encoder}");
            continue;
        }
        let mut rec = Recorder::start_with(codec, 3, &["default_output", "default_input"]);
        std::thread::sleep(Duration::from_secs(5));
        let clip = rec.save();
        let (kinds, duration) = probe(&clip);
        assert_eq!(kinds, ["video", "audio", "audio"], "{codec}:\n{}", rec.log());
        assert_eq!(video_codec(&clip), codec);
        assert!((2.5..=3.9).contains(&duration), "{codec}: {duration} s:\n{}", rec.log());
        let (first, second) = (pitch(&clip, 0), pitch(&clip, 1));
        assert!((425.0..455.0).contains(&first) && (850.0..910.0).contains(&second), "{codec}: {first} Hz and {second} Hz");
        rec.say("stop");
        assert_eq!(rec.finish(), Some(0), "{}", rec.log());
        rec.assert_clean();
    }
}

#[test]
fn the_questions_are_answered_in_the_format_the_app_reads() {
    let ask = |arg: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_zenith-rec")).arg(arg).env("ZENITH_REC_TEST_SOURCE", "1").output().unwrap();
        assert!(out.status.success(), "{arg}");
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(ask("--list-capture-options"), "test|1280x720\n");
    let devices = ask("--list-audio-devices");
    assert!(devices.starts_with("default_output|Default output\ndefault_input|Default input\n"), "{devices}");
    let info = ask("--info");
    assert!(info.starts_with("section=video_codecs\n"), "{info}");
    // One program's sound can be told from another's on Windows only.
    let apps = if cfg!(windows) { "yes" } else { "no" };
    assert!(info.ends_with(&format!("section=system_info\nsupports_app_audio|{apps}\n")), "{info}");
    if have("ffmpeg") {
        assert!(info.contains("\nh264\n"), "{info}");
    }
}

#[test]
fn a_bad_command_line_is_refused_without_recording() {
    let out = Command::new(env!("CARGO_BIN_EXE_zenith-rec")).args(["-w", "screen"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("-o <directory> is needed"));
}
