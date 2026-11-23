//! `zenithd`: with no arguments, the background recorder; with one,
//! a message to the recorder that is already running.
//!
//! The daemon is the same program on Linux and Windows. What differs is
//! kept in a few small places: how the recorder is spoken to (`control`),
//! where requests come in (`server`), signals and sleep (`host`), the save
//! hotkey (`hotkey`), the tray icon (`tray`, `tray_windows`) and how the user is told
//! things (`feedback`).

mod cli;
mod control;
mod daemon;
mod feedback;
mod host;
mod hotkey;
mod logfile;
#[cfg(windows)]
mod overlay;
mod server;
#[cfg(unix)]
mod tray;
#[cfg(windows)]
#[path = "tray_windows.rs"]
mod tray;
mod tray_menu;
#[cfg(windows)]
mod update_window;

pub(crate) use cli::warn;

// On Windows this stays a console program, so that `zenithd status`,
// `save` and `doctor` print where they are run: a program of the windowed
// kind has no output a terminal shows. The price is that whoever starts the
// daemon at login has to start it without a console window.

fn main() -> std::process::ExitCode {
    cli::main()
}
