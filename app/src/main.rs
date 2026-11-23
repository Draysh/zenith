// On Windows this is a program with a window, not one that opens a console.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! `zenith-clips`: the window. The clip library, the editor, export and
//! settings; the recording itself is the daemon's (`zenithd`).

mod bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};

fn main() {
    // `--version` says which version this is and nothing more. A new
    // version is asked this before it takes the old one's place.
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("zenith-clips {}", bridge::updates::VERSION);
        return;
    }
    // `--start-recorder` starts the background recorder and nothing else.
    // On Windows that is how it starts when the user signs in: this program
    // has no console window, and starts the recorder without one too.
    if std::env::args().any(|a| a == "--start-recorder") {
        start_recorder();
        return;
    }
    // A test mode works in a folder of its own, set up before anything is
    // read: never on the user's settings, library or clips.
    let test = test_folder();
    went_back_if_this_version_fails();
    keep_log();
    // On Windows nothing else keeps the recorder running: opening the app
    // starts it when it is not. (A second one notices the first and leaves.)
    if cfg!(windows) && test.is_none() {
        start_recorder();
    }
    // What Qt has to say goes to the terminal, not only to the journal.
    if std::env::var_os("QT_FORCE_STDERR_LOGGING").is_none() {
        // SAFETY: nothing else is running yet.
        unsafe { std::env::set_var("QT_FORCE_STDERR_LOGGING", "1") };
    }
    bridge::library::use_opengl();
    let mut app = QGuiApplication::new();
    QGuiApplication::set_desktop_file_name(&QString::from("zenith-clips"));
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/Zenith/qml/Main.qml"));
    }
    // The window is made: on Windows it gets what a framed window has.
    bridge::library::own_frame();
    // How drawing goes while the window is sized is said in the log.
    bridge::library::watch_sizing();
    if let Some(app) = app.as_mut() {
        let code = app.exec();
        // The window is gone, however it was closed. What it left running
        // is seen to an end before the program is: an export is stopped
        // (ffmpeg with it, and its half-made files removed), and the
        // recorder has been told of settings kept as the window closed.
        // After that nothing is left that leaving at once would cut off.
        bridge::editor::stop_export(std::time::Duration::from_secs(10));
        bridge::settings::wait_for_recorder(std::time::Duration::from_millis(1500));
        if let Some(folder) = test.filter(|t| t.ours) {
            let _ = std::fs::remove_dir_all(folder.root);
        }
        // Closed as it should: a start without this line after it did not.
        eprintln!("{}", zenith_core::applog::closed());
        std::process::exit(code);
    }
}

/// The modes that drive the window without a person and change things as
/// they go (see qml/Main.qml and qml/LibraryTest.qml).
const TEST_MODES: [&str; 4] = ["--selftest", "--scrolltest", "--librarytest", "--resizetest"];

/// Where a test mode works.
struct TestFolder {
    root: std::path::PathBuf,
    /// Made by this run, and removed when it ends. One named by
    /// `ZENITH_TEST_DIR` is the caller's to look into afterwards.
    ours: bool,
}

/// In a test mode, a fresh folder that stands in for everything of the
/// user's: the settings, the library's index, the cache and the clip folder
/// are all in it, and the recorder that is running is not reached. `None`
/// when no test mode was asked for.
///
/// The folder is new in the temporary directory, or the one `ZENITH_TEST_DIR`
/// names. The clip folder in it holds a few copies of the video named after
/// `--librarytest` or `--test-clip`, when there is one, so the test has clips
/// to work on. A folder that cannot be set up ends the program: a test must
/// not go on to the real library instead.
fn test_folder() -> Option<TestFolder> {
    use zenith_core::config::Config;
    use zenith_core::paths;
    let args: Vec<String> = std::env::args().collect();
    if !args.iter().any(|a| TEST_MODES.contains(&a.as_str())) {
        return None;
    }
    let named = std::env::var_os("ZENITH_TEST_DIR").map(std::path::PathBuf::from).filter(|p| p.is_absolute());
    let ours = named.is_none();
    let root = named.unwrap_or_else(|| std::env::temp_dir().join(format!("zenith-clips-test-{}", std::process::id())));
    let clips = root.join("Videos").join("Clips");
    let set_up = || -> std::io::Result<()> {
        std::fs::create_dir_all(&clips)?;
        // The folders `paths` in the core crate looks up, by the variables it reads.
        let folders: &[(&str, &str)] =
            if cfg!(windows) { &[("APPDATA", "config"), ("LOCALAPPDATA", "data")] } else { &[("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache")] };
        for (var, name) in folders {
            let folder = root.join(name);
            std::fs::create_dir_all(&folder)?;
            // SAFETY: nothing else is running yet.
            unsafe { std::env::set_var(var, &folder) };
        }
        // No recorder listens here: the one that is running is left alone.
        // SAFETY: nothing else is running yet.
        unsafe { std::env::set_var("ZENITH_SOCKET", root.join("no-recorder.sock")) };
        // The settings of a first start, but for the clip folder, and
        // without looking for new versions.
        let mut cfg = Config::default();
        cfg.library.dir = clips.clone();
        cfg.updates.check = false;
        cfg.save(&paths::config_file()).map_err(std::io::Error::other)?;
        // The welcome screen has been seen: the test gets the library.
        std::fs::create_dir_all(paths::data_dir())?;
        std::fs::write(paths::data_dir().join("welcomed"), b"")?;
        let video = ["--librarytest", "--test-clip"].iter().find_map(|flag| args.get(args.iter().position(|a| a == flag)? + 1)).filter(|v| !v.starts_with("--"));
        if let Some(video) = video {
            let ending = std::path::Path::new(video).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
            for n in 1..=8 {
                // Named as the recorder names clips, a day apart, in two games.
                let game = if n % 2 == 0 { "Seeded Two" } else { "Seeded One" };
                std::fs::create_dir_all(clips.join(game))?;
                std::fs::copy(video, clips.join(game).join(format!("{game} 2026-01-{n:02} 12-00-00.{ending}")))?;
            }
        }
        Ok(())
    };
    if let Err(e) = set_up() {
        eprintln!("zenith-clips: the test's own folder {} could not be set up: {e}", root.display());
        std::process::exit(2);
    }
    // Checked, not trusted: every place is under the test's folder.
    let places = [paths::config_file(), paths::data_dir(), paths::cache_dir(), Config::load_usable(&paths::config_file()).library.dir];
    if let Some(outside) = places.iter().find(|p| !p.starts_with(&root)) {
        eprintln!("zenith-clips: the test would have used {}, outside its own folder {}", outside.display(), root.display());
        std::process::exit(2);
    }
    eprintln!("zenith-clips: a test mode: everything is in {}", root.display());
    Some(TestFolder { root, ours })
}

/// What the window says from here on is kept in its log file, across
/// starts (see `applog` in the core crate), and this start is marked in
/// it. Started from a terminal, it is said there instead, as before.
fn keep_log() {
    use zenith_core::applog;
    let file = applog::file();
    applog::prepare(&file);
    let kept = bridge::library::keep_log(&QString::from(file.to_string_lossy().as_ref()));
    // In the file every line of Qt's says when.
    if kept && std::env::var_os("QT_MESSAGE_PATTERN").is_none() {
        // SAFETY: nothing else is running yet.
        unsafe { std::env::set_var("QT_MESSAGE_PATTERN", "%{time yyyy-MM-dd hh:mm:ss} %{if-category}%{category}: %{endif}%{message}") };
    }
    eprintln!("{}", applog::started(bridge::updates::VERSION));
}

/// A version that was just installed is on trial at its first start. If it
/// is started again without having run well, the version before it comes
/// back, and that one is started in this one's place.
fn went_back_if_this_version_fails() {
    use zenith_core::install::{self, Here, Start};
    if install::here() != Here::Portable {
        return;
    }
    let Some(exe) = install::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    if let Start::WentBack { to } = install::on_start(dir, bridge::updates::VERSION) {
        eprintln!("zenith-clips: version {} did not run well here; back to {to}", bridge::updates::VERSION);
        let again = std::process::Command::new(&exe).args(std::env::args_os().skip(1)).spawn();
        if again.is_ok() {
            std::process::exit(0);
        }
    }
}

/// Start the background recorder, if it is not running already.
fn start_recorder() {
    use zenith_core::ipc::{self, Request};
    use zenith_core::paths;
    if ipc::ask(&paths::socket(), &Request::Status, std::time::Duration::from_millis(500)).is_ok() {
        return;
    }
    let started = paths::command("zenithd").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    if let Err(e) = started {
        eprintln!("zenith-clips: the recorder could not be started: {e}");
    }
}
