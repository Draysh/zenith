#![cfg(unix)]
//! The daemon run for real, against `tests/fake-gsr` in place of the recorder.
//! Needs ffmpeg (for a sample clip the fake recorder "saves"); skipped without.
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
        eprintln!("skipped: no ffmpeg");
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

    d.write_config("buffer_secs = \"lots\"");
    let Response::Error { message } = d.ask(Request::Reload) else { panic!("bad settings accepted") };
    assert!(message.contains("not valid"), "{message}");
    assert_eq!(d.status().buffer_secs, 30, "the last good settings stay");
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
