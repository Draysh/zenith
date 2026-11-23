//! A replay-buffer recorder that takes `gpu-screen-recorder`'s command line.
//!
//! It writes no video itself: one `ffmpeg` child captures the screen, encodes
//! it and cuts it into one-second pieces in a ring directory, and this
//! program feeds it the sound, throws away pieces older than the buffer,
//! starts it again when it stops, and joins the newest pieces into a clip
//! when `save` arrives on stdin. What is decided here is plain logic that
//! runs and is tested on any system; only the sound devices and the lists of
//! monitors and windows are Windows code.

pub mod args;
pub mod audio;
pub mod diagnose;
pub mod ffmpeg;
pub mod game_window;
pub mod log;
pub mod mixer;
pub mod monitors;
pub mod recorder;
pub mod ring;
pub mod sys;

use std::process::ExitCode;

/// Carry out one command line, and say how it went.
pub fn run(argv: Vec<String>) -> ExitCode {
    let command = match args::parse(&argv) {
        Ok(command) => command,
        Err(err) => {
            eprintln!("zenith-rec: {err}");
            eprintln!("{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    match command {
        args::Command::Record(options) => recorder::run(options),
        args::Command::ListCaptureOptions => {
            for monitor in monitors::list() {
                println!("{}|{}x{}", monitor.name, monitor.size.0, monitor.size.1);
            }
            ExitCode::SUCCESS
        }
        args::Command::ListAudioDevices => {
            for (name, label) in audio::devices() {
                println!("{name}|{label}");
            }
            ExitCode::SUCCESS
        }
        args::Command::Info => {
            print!("{}", diagnose::info());
            ExitCode::SUCCESS
        }
        args::Command::Diagnose => diagnose::run(),
        args::Command::Help => {
            println!("{}", args::USAGE);
            ExitCode::SUCCESS
        }
    }
}
