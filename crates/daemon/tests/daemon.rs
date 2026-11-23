#![cfg(unix)]
//! The daemon run for real, against `tests/fake-gsr` in place of the recorder.
//! Needs ffmpeg (for a sample clip the fake recorder "saves"). Without it
//! every test here is skipped, which is said on the error output; with
//! `ZENITH_REQUIRE_FFMPEG=1` in the environment they fail instead, for
//! where a green run has to mean they ran.
//! The last tests run it against `tests/fake-rec`, which is told what to do
//! over its input the way the Windows recorder is.

use zenith_core::ipc::{IpcError, Request, Response, Status, ask};
use zenith_core::library::{Filter, Library};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Running {
    dir: tempfile::TempDir,
    child: Child,
}

impl Running {
    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn ask(&self, request: Request) -> Response {
        ask(&self.path("run/zenith-clips.sock"), &request, Duration::from_secs(30)).unwrap()
    }

    fn status(&self) -> Status {
        match self.ask(Request::Status) {
            Response::Status(s) => s,
            other => panic!("{other:?}"),
        }
    }

    fn wait_for(&self, what: &str, check: impl Fn(&Status) -> bool) -> Status {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let status = self.status();
            if check(&status) {
                return status;
            }
            assert!(Instant::now() < until, "never {what}: {status:?}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn recorder_args(&self) -> String {
        std::fs::read_to_string(self.path("args")).unwrap_or_default()
    }

    fn write_config(&self, capture: &str) {
        let text = format!(
            "[capture]\n{capture}\n[library]\ndir = \"{}\"\n[feedback]\nsound = false\nnotification = false\n",
            self.path("clips").display()
        );
        std::fs::create_dir_all(self.path("config/zenith-clips")).unwrap();
        std::fs::write(self.path("config/zenith-clips/config.toml"), text).unwrap();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Asked to quit, so that it stops its recorder; killed if it will not.
        let _ = ask(&self.path("run/zenith-clips.sock"), &Request::Quit, Duration::from_secs(5));
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until && matches!(self.child.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The stand-in recorders still running that write into `dir`.
fn recorders(dir: &Path) -> Vec<u32> {
    let wanted = dir.join("clips/.incoming");
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let line = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let line = String::from_utf8_lossy(&line);
            (line.contains("fake-gsr") || line.contains("fake-rec")) && line.contains(&*wanted.to_string_lossy())
        })
        .collect()
}

fn daemon_command(dir: &Path, mode: &str) -> Command {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake-gsr");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zenithd"));
    cmd.env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("ZENITH_RECORDER", fake)
        .env("FAKE_GSR_SAMPLE", dir.join("sample.mp4"))
        .env("FAKE_GSR_ARGS", dir.join("args"))
        .env("FAKE_REC_ENDED", dir.join("ended"))
        .env("FAKE_GSR_MODE", mode)
        .env("ZENITH_NO_TRAY", "1")
        // Nor a save hotkey: asking for one would reach the real desktop
        // of whoever runs the tests.
        .env("ZENITH_NO_HOTKEY", "1")
        .stdin(Stdio::null());
    cmd
}

/// A daemon in a world of its own, or `None` where ffmpeg is missing.
fn start(capture: &str, mode: &str) -> Option<Running> {
    start_with(capture, mode, &[])
}

/// The same, with more in its environment.
fn start_with(capture: &str, mode: &str, env: &[(&str, &str)]) -> Option<Running> {
    let dir = tempfile::tempdir().unwrap();
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30"])
        .args(["-f", "lavfi", "-i", "sine", "-t", "2", "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"])
        .arg(dir.path().join("sample.mp4"))
        .status();
    if !made.is_ok_and(|s| s.success()) {
        let required = std::env::var("ZENITH_REQUIRE_FFMPEG").is_ok_and(|v| !v.is_empty() && v != "0");
        assert!(!required, "ffmpeg is not installed (or could not make a sample clip), and ZENITH_REQUIRE_FFMPEG says it must be");
        // Written straight to the error output, which the test runner
        // does not keep back the way it does what a passing test prints.
        use std::io::Write;
        let name = std::thread::current().name().unwrap_or("a daemon test").to_string();
        let _ = writeln!(std::io::stderr(), "NOTE: {name} was SKIPPED, not passed: ffmpeg is not installed. Set ZENITH_REQUIRE_FFMPEG=1 to fail instead.");
        return None;
    }
    std::fs::create_dir_all(dir.path().join("run")).unwrap();
    let mut running = Running { child: daemon_command(dir.path(), mode).spawn().unwrap(), dir };
    running.write_config(capture);
    // Started before its settings were written: the same as a first run.
    // Killed outright, it leaves its socket behind for the next to clear away.
    let _ = running.child.kill();
    let _ = running.child.wait();
    running.child = daemon_command(running.dir.path(), mode).envs(env.iter().copied()).stderr(Stdio::null()).spawn().unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while ask(&running.path("run/zenith-clips.sock"), &Request::Status, Duration::from_secs(1)).is_err() {
        assert!(Instant::now() < until, "the daemon never answered");
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(running)
}

#[test]
fn the_shortcut_saves_a_clip_into_the_library() {
    let Some(d) = start("activation = \"always\"\nbuffer_secs = 45", "") else { return };
    let status = d.wait_for("capturing", |s| s.state == "capturing");
    assert_eq!((status.buffer_secs, status.clips_saved, status.game), (45, 0, None));
    d.wait_for("the recorder's command line", |_| d.recorder_args().contains("-o "));
    let args = d.recorder_args();
    assert!(args.contains("-r 45") && args.contains("-replay-storage ram"), "{args}");
    assert!(args.contains(&format!("-o {}", d.path("clips/.incoming").display())), "{args}");

    let Response::Saved { path } = d.ask(Request::Save) else { panic!("not saved") };
    assert!(path.starts_with(d.path("clips/Desktop")) && path.is_file(), "{}", path.display());
    assert!(path.file_name().unwrap().to_string_lossy().starts_with("Desktop 20"));
    assert_eq!(std::fs::read_dir(d.path("clips/.incoming")).unwrap().count(), 0, "nothing is left unfiled");

    let second = d.ask(Request::Save);
    assert!(matches!(second, Response::Saved { .. }), "{second:?}");
    let status = d.status();
    assert_eq!((status.state.as_str(), status.clips_saved), ("capturing", 2));

    let library = Library::open(&d.path("data/zenith-clips/library.db")).unwrap();
    let clips = library.list(&Filter::default()).unwrap();
    assert_eq!(clips.len(), 2);
    assert_eq!((clips[0].game.as_str(), clips[0].width, clips[0].height), ("Desktop", 320, 180));
    assert!((clips[0].seconds - 2.0).abs() < 0.2);
}

#[test]
fn capture_follows_the_switch_and_a_press_without_it_says_why() {
    let Some(d) = start("activation = \"game\"", "") else { return };
    // No game is running in this world, so nothing is captured.
    assert_eq!(d.status().state, "idle");
    let Response::Error { message } = d.ask(Request::Save) else { panic!("saved with nothing running") };
    assert!(message.contains("Nothing to clip"), "{message}");

    assert_eq!(d.ask(Request::Capture { force: Some(true) }), Response::Ok);
    d.wait_for("capturing", |s| s.state == "capturing" && s.forced == Some(true));
    assert!(matches!(d.ask(Request::Save), Response::Saved { .. }));
    assert_eq!(d.ask(Request::Capture { force: None }), Response::Ok);
    assert_eq!(d.status().state, "idle");
}

#[test]
fn new_settings_restart_the_recorder() {
    let Some(d) = start("activation = \"always\"\nbuffer_secs = 60\nfps = 60", "") else { return };
    d.wait_for("capturing", |s| s.state == "capturing");
    d.wait_for("the first recorder", |_| d.recorder_args().contains("-r 60"));
    d.write_config("activation = \"always\"\nbuffer_secs = 30\nfps = 30\nresolution = \"720p\"");
    assert_eq!(d.ask(Request::Reload), Response::Ok);
    d.wait_for("capturing with the new settings", |s| {
        s.state == "capturing" && s.buffer_secs == 30 && d.recorder_args().contains("-r 30")
    });
    let args = d.recorder_args();
    assert!(args.contains("-r 30") && args.contains("-f 30") && args.contains("-s 1280x720") && args.contains("-w DP-1"), "{args}");
    assert!(matches!(d.ask(Request::Save), Response::Saved { .. }));

    // One value that is not valid is passed over, as it is at the start:
    // it is at its default, and the rest of the file counts.
    d.write_config("activation = \"always\"\nbuffer_secs = \"lots\"\nfps = 30\nresolution = \"720p\"");
    assert_eq!(d.ask(Request::Reload), Response::Ok);
    d.wait_for("capturing with what could be read", |s| s.state == "capturing" && s.buffer_secs == 60 && d.recorder_args().contains("-r 60"));
    let args = d.recorder_args();
    assert!(args.contains("-f 30") && args.contains("-s 1280x720"), "{args}");
    // And a later change is taken in, with that value still not valid.
    d.write_config("activation = \"always\"\nbuffer_secs = \"lots\"\nfps = 60");
    assert_eq!(d.ask(Request::Reload), Response::Ok);
    d.wait_for("the next change", |s| s.state == "capturing" && d.recorder_args().contains("-f 60"));

    // A file that cannot be read at all is refused.
    std::fs::write(d.path("config/zenith-clips/config.toml"), "[capture\nbuffer_secs = 30\n").unwrap();
    let Response::Error { message } = d.ask(Request::Reload) else { panic!("bad settings accepted") };
    assert!(message.contains("not valid"), "{message}");
    assert_eq!(d.status().buffer_secs, 60, "the last good settings stay");
}

/// A stand-in recorder for one test: it answers what `fake-gsr` answers
/// about the machine, finds its `-o` folder, and then does `body`.
fn recorder_that(body: &str) -> (tempfile::TempDir, String) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // Named as the other, which is what `recorders` looks for.
    let file = dir.path().join("fake-gsr");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --list-capture-options ] && {{ echo \"DP-1|2560x1440\"; exit 0; }}\n[ \"$1\" = --list-audio-devices ] && {{ echo \"default_output|Default output\"; exit 0; }}\nout=\"\"\nwhile [ $# -gt 1 ]; do [ \"$1\" = -o ] && out=\"$2\"; shift; done\n{body}\n"
    );
    std::fs::write(&file, script).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = file.to_string_lossy().into_owned();
    (dir, path)
}

/// What every stand-in does between signals.
const IDLE: &str = "while :; do sleep 0.05 & wait $!; done";

#[test]
fn a_recorder_that_prints_its_clip_and_ends_at_once_does_not_lose_it() {
    let save = "trap 'cp \"$FAKE_GSR_SAMPLE\" \"$out/a.mp4\" && echo \"$out/a.mp4\"; exit 0' USR1; trap 'exit 0' INT TERM";
    let (_script, recorder) = recorder_that(&format!("{save}\n{IDLE}"));
    let Some(d) = start_with("activation = \"always\"", "", &[("ZENITH_RECORDER", &recorder)]) else { return };
    for clip in 1..=3 {
        d.wait_for("capturing", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
        let answer = d.ask(Request::Save);
        let Response::Saved { path } = answer else { panic!("clip {clip} was lost: {answer:?}") };
        assert!(path.is_file(), "{}", path.display());
    }
    assert_eq!(d.status().clips_saved, 3);
}

#[test]
fn a_clip_being_written_is_finished_before_quitting() {
    let save = "trap 'sleep 1; cp \"$FAKE_GSR_SAMPLE\" \"$out/a.mp4\" && echo \"$out/a.mp4\"' USR1; trap 'exit 0' INT TERM";
    let (_script, recorder) = recorder_that(&format!("{save}\n{IDLE}"));
    let Some(mut d) = start_with("activation = \"always\"", "", &[("ZENITH_RECORDER", &recorder)]) else { return };
    d.wait_for("capturing", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    let socket = d.path("run/zenith-clips.sock");
    let saving = std::thread::spawn(move || ask(&socket, &Request::Save, Duration::from_secs(30)));
    d.wait_for("saving", |s| s.state == "saving");
    assert_eq!(d.ask(Request::Quit), Response::Ok);
    let answer = saving.join().unwrap().unwrap();
    let Response::Saved { path } = answer else { panic!("the clip was thrown away: {answer:?}") };
    assert!(path.starts_with(d.path("clips/Desktop")) && path.is_file(), "{}", path.display());
    assert!(d.child.wait().unwrap().success());
    assert!(recorders(d.dir.path()).is_empty());
    assert!(!d.path("clips/.incoming").exists());
    let library = Library::open(&d.path("data/zenith-clips/library.db")).unwrap();
    assert_eq!(library.list(&Filter::default()).unwrap().len(), 1);
}

#[test]
fn a_clip_that_cannot_be_read_is_still_saved() {
    let save = "trap 'echo not a video > \"$out/a.mp4\"; echo \"$out/a.mp4\"' USR1; trap 'exit 0' INT TERM";
    let (_script, recorder) = recorder_that(&format!("{save}\n{IDLE}"));
    let Some(d) = start_with("activation = \"always\"", "", &[("ZENITH_RECORDER", &recorder)]) else { return };
    d.wait_for("capturing", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    let answer = d.ask(Request::Save);
    let Response::Saved { path } = answer else { panic!("the file is there, and it said {answer:?}") };
    assert!(path.starts_with(d.path("clips/Desktop")));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a video\n");
    assert_eq!(d.status().clips_saved, 1);
    // Not in the index, which has nothing to say about it: the app takes
    // it in when it looks through the folder and can read it.
    let library = Library::open(&d.path("data/zenith-clips/library.db")).unwrap();
    assert!(library.list(&Filter::default()).unwrap().is_empty());
}

#[test]
fn a_recorder_that_outlived_its_daemon_is_stopped_by_the_next() {
    // One the system's "end with your parent" does not reach, as it does
    // not reach a gpu-screen-recorder that was given capabilities.
    let (_script, recorder) = recorder_that(&format!("trap '' TERM; trap 'exit 0' INT\n{IDLE}"));
    let Some(mut d) = start_with("activation = \"always\"", "", &[("ZENITH_RECORDER", &recorder)]) else { return };
    d.wait_for("a recorder", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    let before = recorders(d.dir.path());
    d.child.kill().unwrap();
    d.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(recorders(d.dir.path()), before, "this one lives on");

    d.child = daemon_command(d.dir.path(), "").env("ZENITH_RECORDER", &recorder).stderr(Stdio::null()).spawn().unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while ask(&d.path("run/zenith-clips.sock"), &Request::Status, Duration::from_secs(1)).is_err() {
        assert!(Instant::now() < until, "the next daemon never answered");
        std::thread::sleep(Duration::from_millis(20));
    }
    d.wait_for("its own recorder", |s| s.state == "capturing" && recorders(d.dir.path()).iter().any(|pid| !before.contains(pid)));
    let now = recorders(d.dir.path());
    assert!(now.len() == 1 && !before.contains(&now[0]), "the one left behind is gone: {before:?} then {now:?}");
}

#[test]
fn a_recorder_that_dies_is_restarted_and_the_reason_kept() {
    let Some(d) = start("activation = \"always\"", "crash") else { return };
    let status = d.wait_for("restarting", |s| s.state == "restarting");
    assert_eq!(status.detail.as_deref(), Some("attempt 1"));
    let Response::Error { message } = d.ask(Request::Save) else { panic!("saved without a recorder") };
    assert!(message.contains("Nothing to clip"), "{message}");
    d.wait_for("a second attempt", |s| s.detail.as_deref() == Some("attempt 2"));
}

#[test]
fn a_recorder_that_does_not_answer_is_started_afresh() {
    let Some(d) = start_with("activation = \"always\"", "deaf", &[("ZENITH_SAVE_TIMEOUT", "0.5")]) else { return };
    d.wait_for("capturing", |s| s.state == "capturing");
    let first = d.wait_for("a recorder", |_| !recorders(d.dir.path()).is_empty());
    let before = recorders(d.dir.path());
    assert_eq!((first.state.as_str(), before.len()), ("capturing", 1));
    let Response::Error { message } = d.ask(Request::Save) else { panic!("a deaf recorder saved") };
    assert!(message.contains("not saved") && message.contains("in time"), "{message}");
    d.wait_for("another recorder", |s| s.state == "capturing" && recorders(d.dir.path()).iter().any(|pid| !before.contains(pid)));
    assert_eq!(recorders(d.dir.path()).len(), 1, "the one that did not answer is gone");
}

#[test]
fn a_game_starts_capture_and_its_end_leaves_time_to_clip_it() {
    let Some(d) = start_with("activation = \"game\"", "", &[("ZENITH_GAME_GRACE", "4")]) else { return };
    assert_eq!(d.status().state, "idle");
    // Steam is installed after the daemon started, then a game of it runs.
    let apps = d.path(".local/share/Steam/steamapps");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(apps.join("appmanifest_999999990.acf"), "\"AppState\"\n{\n\t\"name\"\t\t\"Test: Game\"\n}\n").unwrap();
    let mut game = Command::new("sleep").arg("60").env("SteamAppId", "999999990").spawn().unwrap();
    d.wait_for("capturing the game", |s| s.state == "capturing" && s.game.as_deref() == Some("Test: Game"));

    game.kill().unwrap();
    game.wait().unwrap();
    let after = d.wait_for("the game's end to be seen", |s| s.game.is_none());
    assert_eq!(after.state, "capturing", "its last moments can still be clipped");
    let Response::Saved { path } = d.ask(Request::Save) else { panic!("not saved") };
    assert!(path.starts_with(d.path("clips/Test Game")), "{}", path.display());
    d.wait_for("capture to stop", |s| s.state == "idle");
}

#[test]
fn a_daemon_that_is_killed_takes_its_recorder_with_it() {
    let Some(mut d) = start("activation = \"always\"", "") else { return };
    d.wait_for("a recorder", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    d.child.kill().unwrap();
    d.child.wait().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while !recorders(d.dir.path()).is_empty() {
        assert!(Instant::now() < until, "the recorder outlived the daemon");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_silent_client_is_let_go_and_the_socket_is_the_users_own() {
    use std::os::unix::fs::PermissionsExt;
    let Some(d) = start("activation = \"game\"", "") else { return };
    let socket = d.path("run/zenith-clips.sock");
    assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
    // Connected and saying nothing: the others are still answered.
    let silent = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    assert_eq!(d.status().state, "idle");
    drop(silent);
}

#[test]
fn one_daemon_at_a_time_and_quit_cleans_up() {
    let Some(mut d) = start("activation = \"always\"", "") else { return };
    d.wait_for("capturing", |s| s.state == "capturing");
    let second = daemon_command(d.dir.path(), "").stderr(Stdio::piped()).output().unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already running"));

    assert_eq!(d.ask(Request::Quit), Response::Ok);
    assert!(d.child.wait().unwrap().success());
    assert!(!d.path("run/zenith-clips.sock").exists());
    assert!(!d.path("clips/.incoming").exists());
    let gone = ask(&d.path("run/zenith-clips.sock"), &Request::Status, Duration::from_secs(1)).unwrap_err();
    assert!(matches!(gone, IpcError::NotRunning));
}

/// A daemon whose recorder is `tests/fake-rec`, spoken to over its input.
fn start_on_stdin(mode: &str) -> Option<Running> {
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake-rec");
    let env = [("ZENITH_RECORDER", &*fake.to_string_lossy()), ("ZENITH_RECORDER_CONTROL", "stdin"), ("ZENITH_SAVE_TIMEOUT", "0.5")];
    let running = start_with("activation = \"always\"", mode, &env)?;
    Some(running)
}

#[test]
fn a_clip_the_recorder_could_not_write_is_said_at_once_and_the_recorder_kept() {
    // Far longer than the test: an answer that comes is the recorder's own.
    let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fake-rec");
    let env = [("ZENITH_RECORDER", &*fake.to_string_lossy()), ("ZENITH_RECORDER_CONTROL", "stdin"), ("ZENITH_SAVE_TIMEOUT", "600")];
    let Some(d) = start_with("activation = \"always\"", "failing", &env) else { return };
    d.wait_for("a recorder", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    let before = recorders(d.dir.path());
    let Response::Error { message } = d.ask(Request::Save) else { panic!("a clip that could not be written was saved") };
    assert!(message.contains("could not be joined"), "{message}");
    assert_eq!(d.status().state, "capturing");
    assert_eq!(recorders(d.dir.path()), before, "the same recorder, still recording");
}

/// Why the stand-in recorder of `d` ended, once it has.
fn ended(d: &Running) -> String {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let why = std::fs::read_to_string(d.path("ended")).unwrap_or_default();
        if !why.is_empty() || Instant::now() >= until {
            return why.trim().to_string();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_recorder_told_over_its_input_saves_clips_and_stops_when_told() {
    let Some(mut d) = start_on_stdin("") else { return };
    d.wait_for("capturing", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    let Response::Saved { path } = d.ask(Request::Save) else { panic!("not saved") };
    assert!(path.starts_with(d.path("clips/Desktop")) && path.is_file(), "{}", path.display());
    assert!(matches!(d.ask(Request::Save), Response::Saved { .. }));
    assert_eq!(d.status().clips_saved, 2);

    assert_eq!(d.ask(Request::Quit), Response::Ok);
    assert!(d.child.wait().unwrap().success());
    assert_eq!(ended(&d), "stop", "it was asked to, not killed");
    assert!(recorders(d.dir.path()).is_empty());
}

#[test]
fn a_recorder_told_over_its_input_that_does_not_answer_is_started_afresh() {
    let Some(d) = start_on_stdin("deaf") else { return };
    d.wait_for("a recorder", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    let before = recorders(d.dir.path());
    let Response::Error { message } = d.ask(Request::Save) else { panic!("a deaf recorder saved") };
    assert!(message.contains("not saved") && message.contains("in time"), "{message}");
    d.wait_for("another recorder", |s| s.state == "capturing" && recorders(d.dir.path()).iter().any(|pid| !before.contains(pid)));
    assert_eq!(recorders(d.dir.path()).len(), 1, "the one that did not answer is gone");
}

#[test]
fn a_recorder_told_over_its_input_ends_when_the_daemon_is_killed() {
    let Some(mut d) = start_on_stdin("") else { return };
    d.wait_for("a recorder", |s| s.state == "capturing" && !recorders(d.dir.path()).is_empty());
    d.child.kill().unwrap();
    d.child.wait().unwrap();
    // No signal reaches this one: its input closing is all that tells it.
    assert_eq!(ended(&d), "closed");
    let until = Instant::now() + Duration::from_secs(5);
    while !recorders(d.dir.path()).is_empty() {
        assert!(Instant::now() < until, "the recorder outlived the daemon");
        std::thread::sleep(Duration::from_millis(20));
    }
}
