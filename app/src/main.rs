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
    went_back_if_this_version_fails();
    keep_log();
    // On Windows nothing else keeps the recorder running: opening the app
    // starts it when it is not. (A second one notices the first and leaves.)
    if cfg!(windows) {
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
        // Closed as it should: a start without this line after it did not.
        eprintln!("{}", zenith_core::applog::closed());
        std::process::exit(code);
    }
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
