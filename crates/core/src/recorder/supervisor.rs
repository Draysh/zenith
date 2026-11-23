//! What the daemon does with the recorder, as a state machine with no I/O:
//! events go in, the actions to carry out come back. The daemon owns the
//! process, the timers and the signals; every decision is here, and tested.

use std::path::PathBuf;
use std::time::Duration;

/// How long a save may take before it is called failed, whatever its size.
pub const SAVE_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a save of a buffer of `megabytes` may take: the fixed part,
/// and time to write it to a disk that manages 20 MB a second.
pub fn save_timeout(megabytes: u32) -> Duration {
    SAVE_TIMEOUT + Duration::from_secs(megabytes as u64 / 20)
}
/// A recorder that lives this long is counted as having started properly.
pub const HEALTHY_AFTER: Duration = Duration::from_secs(10);
/// After this many failed starts in a row it stops trying.
pub const MAX_ATTEMPTS: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not wanted: no game is running, or capture is switched off.
    Idle,
    /// Running and holding a replay buffer.
    Capturing,
    /// Running, and asked to write the buffer out.
    Saving,
    /// It stopped by itself; a restart is scheduled.
    Restarting { attempt: u32 },
    /// It would not stay up. Stays here until asked to start again.
    Failed { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Capture is wanted or not (a game started or ended, the tray toggle).
    Want(bool),
    /// The settings changed: a running recorder must be started afresh.
    Reconfigure,
    /// The machine woke up or the monitors changed.
    Refresh,
    /// The shortcut.
    SaveRequested,
    /// The recorder reported a finished clip.
    Saved(PathBuf),
    SaveTimedOut,
    /// The recorder process ended; `ran` is how long it lived.
    Exited { ran: Duration, detail: String },
    /// It could not be started at all (not installed, not executable).
    SpawnFailed(String),
    RetryDue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Spawn,
    /// Stop the process and wait for it.
    Kill,
    /// Tell the process to write its buffer out, and start the save timer.
    SignalSave,
    RetryIn(Duration),
    /// A clip is on disk.
    ClipSaved(PathBuf),
    /// Something the user should be told.
    Report(Problem),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The shortcut was pressed while nothing was being captured.
    NotCapturing,
    SaveFailed(String),
    RecorderFailed(String),
}

#[derive(Debug)]
pub struct Supervisor {
    state: State,
    wanted: bool,
    /// Starts in a row that ended before `HEALTHY_AFTER`.
    short_runs: u32,
    /// New settings came while a clip was being written: the recorder is
    /// started afresh once the clip is out.
    restart_pending: bool,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self { state: State::Idle, wanted: false, short_runs: 0, restart_pending: false }
    }
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500 * 2u64.pow(attempt.min(5)))
}

impl Supervisor {
    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn wanted(&self) -> bool {
        self.wanted
    }

    fn running(&self) -> bool {
        matches!(self.state, State::Capturing | State::Saving)
    }

    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        use Action::*;
        use State::*;
        match event {
            Event::Want(true) => {
                self.wanted = true;
                match self.state {
                    // A new request clears an earlier failure.
                    Idle | Failed { .. } => {
                        self.short_runs = 0;
                        self.state = Capturing;
                        vec![Spawn]
                    }
                    _ => vec![],
                }
            }
            Event::Want(false) => {
                self.wanted = false;
                // A save in flight is let finish; `Saved` stops it after.
                if self.state == Saving {
                    return vec![];
                }
                let was_running = self.running();
                self.restart_pending = false;
                self.state = Idle;
                if was_running { vec![Kill] } else { vec![] }
            }
            Event::Reconfigure | Event::Refresh => match self.state {
                Capturing => vec![Kill, Spawn],
                Failed { .. } if self.wanted => {
                    self.short_runs = 0;
                    self.state = Capturing;
                    vec![Spawn]
                }
                // The clip being written comes first; `Saved` restarts it after.
                Saving => {
                    self.restart_pending = true;
                    vec![]
                }
                // Restarting or idle: the next start reads the new settings.
                _ => vec![],
            },
            Event::SaveRequested => match self.state {
                Capturing => {
                    self.state = Saving;
                    vec![SignalSave]
                }
                // A second press while one is being written is the same clip.
                Saving => vec![],
                _ => vec![Report(Problem::NotCapturing)],
            },
            Event::Saved(path) => {
                if self.state != Saving {
                    return vec![ClipSaved(path)];
                }
                let restart = std::mem::take(&mut self.restart_pending);
                if !self.wanted {
                    self.state = Idle;
                    vec![ClipSaved(path), Kill]
                } else if restart {
                    self.state = Capturing;
                    vec![ClipSaved(path), Kill, Spawn]
                } else {
                    self.state = Capturing;
                    vec![ClipSaved(path)]
                }
            }
            Event::SaveTimedOut => {
                if self.state != Saving {
                    return vec![];
                }
                self.restart_pending = false;
                let report = Report(Problem::SaveFailed("the recorder did not write the clip in time".into()));
                // It is not answering: start it afresh rather than trust it.
                if self.wanted {
                    self.state = Capturing;
                    vec![report, Kill, Spawn]
                } else {
                    self.state = Idle;
                    vec![report, Kill]
                }
            }
            Event::Exited { ran, detail } => {
                if !self.running() {
                    return vec![];
                }
                self.restart_pending = false;
                let mut out = vec![];
                if self.state == Saving {
                    out.push(Report(Problem::SaveFailed("the recorder stopped while saving".into())));
                }
                if !self.wanted {
                    self.state = Idle;
                    return out;
                }
                self.short_runs = if ran >= HEALTHY_AFTER { 1 } else { self.short_runs + 1 };
                let attempt = self.short_runs;
                if attempt > MAX_ATTEMPTS {
                    self.state = Failed { reason: detail.clone() };
                    out.push(Report(Problem::RecorderFailed(detail)));
                } else {
                    self.state = Restarting { attempt };
                    out.push(RetryIn(backoff(attempt)));
                }
                out
            }
            Event::SpawnFailed(reason) => {
                self.state = Failed { reason: reason.clone() };
                vec![Report(Problem::RecorderFailed(reason))]
            }
            Event::RetryDue => match self.state {
                Restarting { .. } if self.wanted => {
                    self.state = Capturing;
                    vec![Spawn]
                }
                Restarting { .. } => {
                    self.state = Idle;
                    vec![]
                }
                _ => vec![],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Action::*;
    use super::*;

    fn exited(secs: u64) -> Event {
        Event::Exited { ran: Duration::from_secs(secs), detail: "exit status 1".into() }
    }

    fn capturing() -> Supervisor {
        let mut s = Supervisor::default();
        assert_eq!(s.handle(Event::Want(true)), [Spawn]);
        s
    }

    #[test]
    fn it_runs_only_while_wanted() {
        let mut s = capturing();
        assert_eq!(s.handle(Event::Want(true)), [], "asked twice starts once");
        assert_eq!(s.handle(Event::Want(false)), [Kill]);
        assert_eq!(*s.state(), State::Idle);
        assert_eq!(s.handle(Event::Want(false)), []);
        // Its own exit after being stopped is not a crash.
        assert_eq!(s.handle(exited(30)), []);
    }

    #[test]
    fn a_save_goes_out_and_comes_back() {
        let mut s = capturing();
        assert_eq!(s.handle(Event::SaveRequested), [SignalSave]);
        assert_eq!(s.handle(Event::SaveRequested), [], "a second press is the same clip");
        assert_eq!(s.handle(Event::Saved("/c/a.mp4".into())), [ClipSaved("/c/a.mp4".into())]);
        assert_eq!(*s.state(), State::Capturing);
        assert_eq!(s.handle(Event::SaveRequested), [SignalSave]);
    }

    #[test]
    fn a_press_with_nothing_running_is_reported() {
        let mut s = Supervisor::default();
        assert_eq!(s.handle(Event::SaveRequested), [Report(Problem::NotCapturing)]);
    }

    #[test]
    fn a_game_closing_mid_save_keeps_the_clip() {
        let mut s = capturing();
        s.handle(Event::SaveRequested);
        assert_eq!(s.handle(Event::Want(false)), [], "the save is let finish");
        assert_eq!(s.handle(Event::Saved("/c/a.mp4".into())), [ClipSaved("/c/a.mp4".into()), Kill]);
        assert_eq!(*s.state(), State::Idle);
    }

    #[test]
    fn a_save_that_never_finishes_restarts_the_recorder() {
        let mut s = capturing();
        s.handle(Event::SaveRequested);
        let out = s.handle(Event::SaveTimedOut);
        assert!(matches!(out[0], Report(Problem::SaveFailed(_))));
        assert_eq!(out[1..], [Kill, Spawn]);
        assert_eq!(*s.state(), State::Capturing);
        assert_eq!(s.handle(Event::SaveTimedOut), [], "a late timer does nothing");
    }

    #[test]
    fn a_crash_is_retried_with_growing_waits_then_given_up_on() {
        let mut s = capturing();
        let mut waits = vec![];
        for attempt in 1..=MAX_ATTEMPTS {
            match s.handle(exited(0)).as_slice() {
                [RetryIn(d)] => waits.push(*d),
                other => panic!("attempt {attempt}: {other:?}"),
            }
            assert_eq!(*s.state(), State::Restarting { attempt });
            assert_eq!(s.handle(Event::RetryDue), [Spawn]);
        }
        assert!(waits.windows(2).all(|w| w[1] > w[0]), "{waits:?}");
        assert_eq!(s.handle(exited(0)), [Report(Problem::RecorderFailed("exit status 1".into()))]);
        assert!(matches!(s.state(), State::Failed { .. }));
        // Asking again, or new settings, tries afresh.
        assert_eq!(s.handle(Event::Reconfigure), [Spawn]);
        assert_eq!(s.handle(exited(0)), [RetryIn(backoff(1))]);
    }

    #[test]
    fn a_long_run_resets_the_count() {
        let mut s = capturing();
        for _ in 0..3 {
            s.handle(exited(0));
            s.handle(Event::RetryDue);
        }
        assert_eq!(s.handle(exited(600)), [RetryIn(backoff(1))]);
    }

    #[test]
    fn stopping_during_a_wait_cancels_the_restart() {
        let mut s = capturing();
        s.handle(exited(0));
        assert_eq!(s.handle(Event::Want(false)), []);
        assert_eq!(s.handle(Event::RetryDue), []);
        assert_eq!(*s.state(), State::Idle);
    }

    #[test]
    fn dying_mid_save_reports_the_lost_clip() {
        let mut s = capturing();
        s.handle(Event::SaveRequested);
        let out = s.handle(exited(60));
        assert!(matches!(out[0], Report(Problem::SaveFailed(_))));
        assert_eq!(out[1], RetryIn(backoff(1)));
    }

    #[test]
    fn new_settings_restart_a_running_recorder_only() {
        let mut s = Supervisor::default();
        assert_eq!(s.handle(Event::Reconfigure), []);
        s.handle(Event::Want(true));
        assert_eq!(s.handle(Event::Reconfigure), [Kill, Spawn]);
        assert_eq!(s.handle(Event::Refresh), [Kill, Spawn]);
    }

    #[test]
    fn new_settings_during_a_save_wait_for_the_clip() {
        let mut s = capturing();
        s.handle(Event::SaveRequested);
        assert_eq!(s.handle(Event::Reconfigure), [], "the clip is not interrupted");
        assert_eq!(s.handle(Event::Saved("/c/a.mp4".into())), [ClipSaved("/c/a.mp4".into()), Kill, Spawn]);
        assert_eq!(*s.state(), State::Capturing);
        // Only once: the next clip does not restart it again.
        s.handle(Event::SaveRequested);
        assert_eq!(s.handle(Event::Saved("/c/b.mp4".into())), [ClipSaved("/c/b.mp4".into())]);
        // No longer wanted by the time the clip is out: it is stopped, not restarted.
        s.handle(Event::SaveRequested);
        s.handle(Event::Refresh);
        s.handle(Event::Want(false));
        assert_eq!(s.handle(Event::Saved("/c/c.mp4".into())), [ClipSaved("/c/c.mp4".into()), Kill]);
    }

    #[test]
    fn a_bigger_buffer_gets_longer_to_be_written() {
        assert_eq!(save_timeout(0), SAVE_TIMEOUT);
        assert_eq!(save_timeout(168), Duration::from_secs(28));
        assert!(save_timeout(3700) > Duration::from_secs(180));
    }

    #[test]
    fn a_missing_recorder_fails_at_once() {
        let mut s = capturing();
        assert_eq!(s.handle(Event::SpawnFailed("not installed".into())), [Report(Problem::RecorderFailed("not installed".into()))]);
        assert_eq!(s.handle(Event::SaveRequested), [Report(Problem::NotCapturing)]);
        assert_eq!(s.handle(Event::Want(true)), [Spawn]);
    }
}
