//! The command line: with no arguments, the background recorder; with
//! one, a message to the recorder that is already running.

use crate::{control, daemon, feedback, logfile};
use zenith_core::ipc::{self, IpcError, Request, Response};
use zenith_core::{paths, recorder};
use std::process::ExitCode;
use std::time::Duration;

const HELP: &str = "\
zenithd                    run the recorder in the background
zenithd save               save the last seconds as a clip (bind this to a key)
zenithd status             what it is doing
zenithd capture on|off|auto   switch capture on or off, or leave it to the settings
zenithd reload             read the settings file again
zenithd quit               stop the recorder
zenithd doctor             check that everything needed to clip is in place
zenithd report             how things stand, to paste to whoever is helping";

fn request(args: &[&str]) -> Option<Request> {
    Some(match args {
        ["save"] => Request::Save,
        ["status"] => Request::Status,
        ["capture", "on"] => Request::Capture { force: Some(true) },
        ["capture", "off"] => Request::Capture { force: Some(false) },
        ["capture", "auto"] => Request::Capture { force: None },
        ["reload"] => Request::Reload,
        ["quit"] => Request::Quit,
        _ => return None,
    })
}

/// Say something on the error output. A terminal that has gone away is not
/// an error, and must never end the recorder. The daemon's lines also go to
/// its log file, where it keeps one (`logfile`).
pub(crate) fn warn(text: impl std::fmt::Display) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{text}");
    logfile::line(&text);
}

/// A number of seconds from the environment, for the tests to shorten a wait.
fn seconds(var: &str) -> Option<Duration> {
    std::env::var(var).ok()?.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0).map(Duration::from_secs_f64)
}

/// Print a line; a reader that has gone away (`| head`) is not an error.
fn say(text: impl std::fmt::Display) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{text}");
}

fn send(request: Request) -> ExitCode {
    // A save waits for the clip to be written, which a long buffer on a
    // slow disk takes minutes over.
    let wait = Duration::from_secs(if request == Request::Save { 600 } else { 30 });
    match ipc::ask(&paths::socket(), &request, wait) {
        Ok(Response::Ok) => ExitCode::SUCCESS,
        Ok(Response::Saved { path }) => {
            say(path.display());
            ExitCode::SUCCESS
        }
        Ok(Response::Status(s)) => {
            let capture = match s.forced {
                Some(true) => "on",
                Some(false) => "off",
                None => "auto",
            };
            say(format_args!("state: {}{}", s.state, s.detail.map(|d| format!(" ({d})")).unwrap_or_default()));
            say(format_args!("game: {}", s.game.as_deref().unwrap_or("none")));
            say(format_args!("capture: {capture}"));
            if let Some(way) = &s.recording {
                say(format_args!("recording: {way}"));
            }
            say(format_args!("buffer: {} s", s.buffer_secs));
            say(format_args!("clips saved: {}", s.clips_saved));
            ExitCode::SUCCESS
        }
        Ok(Response::Error { message }) => {
            warn(message);
            ExitCode::FAILURE
        }
        Err(e) => {
            warn(&e);
            // The shortcut has no terminal to print to.
            if request == Request::Save && matches!(e, IpcError::NotRunning) {
                let _ = feedback::problem("Zenith is not running", "Start it to clip: zenithd").join();
            }
            ExitCode::FAILURE
        }
    }
}

/// Look the machine over and say what was found.
/// Print the report the app's "Copy report" button copies.
fn report() -> ExitCode {
    use zenith_core::config::Config;
    use zenith_core::doctor::{assess, gather};
    use zenith_core::recorder::outputs;
    use zenith_core::{install, report};
    let cfg = Config::load(&paths::config_file()).unwrap_or_default();
    let program = std::env::var("ZENITH_RECORDER").unwrap_or_else(|_| recorder::PROGRAM.into());
    let checks = assess(&gather(&cfg, &program, &paths::socket()), &cfg);
    let status = match ipc::ask(&paths::socket(), &Request::Status, Duration::from_secs(2)) {
        Ok(Response::Status(status)) => Some(status),
        _ => None,
    };
    say(report::text(&report::Report {
        version: env!("CARGO_PKG_VERSION"),
        system: report::system(),
        installed: install::here().word(),
        status: status.as_ref(),
        checks: &checks,
        monitors: &outputs::list(&program),
        codecs: &outputs::codecs(&program),
        log: &report::log_tail(80),
        window: &report::window_log(150),
    }));
    ExitCode::SUCCESS
}

fn doctor() -> ExitCode {
    use zenith_core::config::Config;
    use zenith_core::doctor::{Level, assess, gather, worst};
    let cfg = Config::load(&paths::config_file()).unwrap_or_else(|e| {
        say(format_args!("! Settings: {e}\n    The defaults are used until the file is fixed."));
        Config::default()
    });
    let program = std::env::var("ZENITH_RECORDER").unwrap_or_else(|_| recorder::PROGRAM.into());
    let checks = assess(&gather(&cfg, &program, &paths::socket()), &cfg);
    for check in &checks {
        let mark = match check.level {
            Level::Ok => "\u{2713}",
            Level::Warning => "!",
            Level::Problem => "\u{2717}",
        };
        say(format_args!("{mark} {}: {}", check.name, check.detail));
        if !check.fix.is_empty() {
            say(format_args!("    {}", check.fix));
        }
    }
    match worst(&checks) {
        Level::Problem => ExitCode::FAILURE,
        _ => ExitCode::SUCCESS,
    }
}

pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    if args.first() == Some(&"ctl") {
        args.remove(0);
    }
    match args.as_slice() {
        [] => {}
        ["-h" | "--help" | "help"] => {
            say(HELP);
            return ExitCode::SUCCESS;
        }
        ["-V" | "--version"] => {
            say(format_args!("zenithd {}", env!("CARGO_PKG_VERSION")));
            return ExitCode::SUCCESS;
        }
        // What the app runs, from a copy of this program, to put a new
        // version in place on Windows (see `update_window`).
        #[cfg(windows)]
        ["update", setup, version, app] => return crate::update_window::run(setup, version, app),
        ["doctor"] => return doctor(),
        ["report"] => return report(),
        other => {
            return match request(other) {
                Some(request) => send(request),
                None => {
                    warn(HELP);
                    ExitCode::from(2)
                }
            };
        }
    }

    let opts = daemon::Options {
        config: paths::config_file(),
        socket: paths::socket(),
        index: paths::data_dir().join("library.db"),
        recorder: std::env::var("ZENITH_RECORDER").unwrap_or_else(|_| recorder::PROGRAM.into()),
        control: control::Control::named(std::env::var("ZENITH_RECORDER_CONTROL").ok().as_deref()),
        save_timeout: seconds("ZENITH_SAVE_TIMEOUT"),
        game_grace: seconds("ZENITH_GAME_GRACE").unwrap_or(daemon::GAME_GRACE),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            warn(format_args!("zenithd: it could not start: {e}"));
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(daemon::run(opts)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            warn(format_args!("zenithd: {e}"));
            ExitCode::FAILURE
        }
    }
}
