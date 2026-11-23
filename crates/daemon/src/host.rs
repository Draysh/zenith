//! What the daemon asks of the system it runs on, where the two differ and
//! the difference is small: being told to stop, and noticing a sleep.

use crate::daemon::Msg;
use std::time::Duration;
use tokio::sync::mpsc;

/// Turn the system's ways of saying "stop" and "read the settings again"
/// into messages: SIGTERM and SIGINT stop it, SIGHUP reloads.
#[cfg(unix)]
pub fn signals(tx: &mpsc::UnboundedSender<Msg>) -> Result<(), String> {
    use tokio::signal::unix::{SignalKind, signal};
    for (kind, reload) in [(SignalKind::terminate(), false), (SignalKind::interrupt(), false), (SignalKind::hangup(), true)] {
        let mut stream = signal(kind).map_err(|e| e.to_string())?;
        let tx = tx.clone();
        tokio::spawn(async move {
            while stream.recv().await.is_some() {
                let _ = tx.send(if reload { Msg::Reload } else { Msg::Shutdown });
            }
        });
    }
    Ok(())
}

/// Windows has no signals. Ctrl+C in a console the daemon was started from
/// stops it; everything else (`zenithd quit`, `zenithd reload`) comes over
/// the pipe.
#[cfg(windows)]
pub fn signals(tx: &mpsc::UnboundedSender<Msg>) -> Result<(), String> {
    let tx = tx.clone();
    tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            let _ = tx.send(Msg::Shutdown);
        }
    });
    Ok(())
}

/// How long this machine has been asleep since it started: the clock that
/// counts sleep, less the one that does not. It grows when it wakes up.
#[cfg(unix)]
pub fn slept() -> Duration {
    let read = |clock| {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `t` is a timespec of ours for the call to fill in.
        unsafe { libc::clock_gettime(clock, &mut t) };
        Duration::new(t.tv_sec.max(0) as u64, t.tv_nsec.clamp(0, 999_999_999) as u32)
    };
    read(libc::CLOCK_BOOTTIME).saturating_sub(read(libc::CLOCK_MONOTONIC))
}

/// On Windows a sleep is not looked for: the recorder there is Zenith's
/// own, and is left to carry on after one. The number never grows, so the
/// daemon never thinks the machine woke up.
#[cfg(windows)]
pub fn slept() -> Duration {
    Duration::ZERO
}
