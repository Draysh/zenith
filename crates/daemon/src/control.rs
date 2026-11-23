//! Telling the recorder what to do. The two recorders take the same command
//! line and differ only in how they are spoken to once they run:
//! gpu-screen-recorder listens for signals; `zenith-rec`, on Windows where
//! there are none, reads a line at a time from its input. This is the one
//! place that knows which is which.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};

/// The line that makes `zenith-rec` write a clip and print its path.
pub const SAVE: &str = "save\n";
/// The line that makes it stop. Its input closing means the same.
pub const STOP: &str = "stop\n";
/// A line is a few bytes into a pipe; a recorder that has stopped reading
/// must not hold the daemon up over one.
const WRITE_WITHIN: Duration = Duration::from_secs(1);

/// How a recorder is spoken to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// SIGUSR1 to save, SIGINT to stop.
    #[cfg(unix)]
    Signals,
    /// `save` and `stop` as lines on its input.
    Stdin,
}

impl Control {
    /// What this system's recorder understands.
    pub fn native() -> Self {
        #[cfg(unix)]
        return Self::Signals;
        #[cfg(windows)]
        return Self::Stdin;
    }

    /// The control a name stands for; anything but `stdin` is this system's
    /// own. The tests use it to run the Windows way on Linux.
    pub fn named(name: Option<&str>) -> Self {
        if name == Some("stdin") { Self::Stdin } else { Self::native() }
    }

    /// The command that runs the recorder `program`, with its input set up
    /// for this control. Its output and arguments are the caller's to add.
    pub fn command(self, program: &str) -> Command {
        #[cfg(unix)]
        let mut command = Command::new(program);
        // On Windows the recorder is beside this program, and must not open
        // a console window of its own.
        #[cfg(windows)]
        let mut command = Command::from(zenith_core::paths::command(program));
        command.stdin(match self {
            #[cfg(unix)]
            Self::Signals => Stdio::null(),
            Self::Stdin => Stdio::piped(),
        });
        // SAFETY: only `prctl`, which may be called between fork and exec.
        // The recorder is told to stop when this process is gone, however
        // it went, so it never records on with nobody to save its clips.
        // (On Windows its input closes with this process, which it takes
        // the same way.) The system forgets this for a program that was
        // given capabilities, as gpu-screen-recorder often is: for that
        // there is `stop_leftover`.
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        command
    }

    /// The way to speak to a recorder just started with [`Self::command`].
    /// `None` when it ended as it started.
    pub fn link(self, child: &mut Child) -> Option<Link> {
        Some(Link { control: self, pid: child.id()?, stdin: child.stdin.take() })
    }
}

/// What tells one process from every other that has had or will have its
/// number: when it started, in the kernel's ticks since this boot, and
/// which boot that was. `None` when it is not there, or cannot be told.
#[cfg(unix)]
fn birth(pid: u32) -> Option<(u64, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the name, which is in brackets and may hold anything: its
    // state comes first and the start time is the twentieth field from
    // there. One that has ended and only waits to be collected is gone.
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
    if matches!(fields.next()?, "Z" | "X") {
        return None;
    }
    let started = fields.nth(18)?.parse().ok()?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some((started, boot.trim().to_string()))
}

/// Where the running recorder is written down, beside the socket.
#[cfg(unix)]
fn recorder_note(socket: &Path) -> std::path::PathBuf {
    socket.with_extension("recorder")
}

/// Write down which process the recorder is, for the daemon after this
/// one to find if this one is killed (`stop_leftover`).
#[cfg(unix)]
pub fn note_recorder(socket: &Path, pid: u32) {
    let Some((started, boot)) = birth(pid) else { return };
    if let Err(e) = std::fs::write(recorder_note(socket), format!("{pid} {started} {boot}\n")) {
        crate::warn(format_args!("zenithd: the recorder could not be written down: {e}"));
    }
}

/// The recorder has ended: there is none to find.
#[cfg(unix)]
pub fn forget_recorder(socket: &Path) {
    let _ = std::fs::remove_file(recorder_note(socket));
}

/// The process a note names, while it is still the one that was written
/// down: the same number, started at the same moment of the same boot.
/// Anything less and it is somebody else's, or nobody's.
#[cfg(unix)]
fn noted(note: &str) -> Option<u32> {
    let mut parts = note.split_whitespace();
    let pid: u32 = parts.next()?.parse().ok()?;
    let started: u64 = parts.next()?.parse().ok()?;
    let boot = parts.next()?;
    // 0 and 1 are never a recorder, and a signal to them goes far.
    (pid > 1 && birth(pid).is_some_and(|(now, this_boot)| now == started && this_boot == boot)).then_some(pid)
}

/// Stop the recorder an earlier daemon left running. A daemon that is
/// killed takes its recorder with it only when the system lets it
/// (`Control::command`); otherwise the recorder records on, and the one
/// this daemon starts would be a second. Only to be called by the one
/// daemon of this socket, and only a process proven to be the one that
/// was written down is signalled.
#[cfg(unix)]
pub async fn stop_leftover(socket: &Path) {
    let file = recorder_note(socket);
    let Ok(note) = std::fs::read_to_string(&file) else { return };
    let _ = std::fs::remove_file(&file);
    let Some(pid) = noted(&note) else { return };
    crate::warn(format_args!("zenithd: the recorder of an earlier run (pid {pid}) is still running; stopping it"));
    // The way gpu-screen-recorder is asked to stop, then the way nothing refuses.
    for (signal, within) in [(libc::SIGINT, Duration::from_secs(3)), (libc::SIGKILL, Duration::from_secs(2))] {
        // Looked at again before each signal: it may have gone meanwhile.
        if noted(&note).is_none() {
            return;
        }
        // SAFETY: a signal to a process just proven to be the recorder.
        unsafe { libc::kill(pid as i32, signal) };
        let until = tokio::time::Instant::now() + within;
        while noted(&note).is_some() && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    if noted(&note).is_some() {
        crate::warn(format_args!("zenithd: the recorder of an earlier run (pid {pid}) would not stop"));
    }
}

/// On Windows the recorder's input closes with the daemon, however it
/// went, and the recorder ends itself: there is nothing to write down.
#[cfg(windows)]
pub fn note_recorder(_socket: &Path, _pid: u32) {}
#[cfg(windows)]
pub fn forget_recorder(_socket: &Path) {}
#[cfg(windows)]
pub async fn stop_leftover(_socket: &Path) {}

/// The daemon's end of a running recorder.
pub struct Link {
    control: Control,
    pid: u32,
    stdin: Option<ChildStdin>,
}

impl Link {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Ask for a clip. Only to be called while the recorder has not been
    /// waited for: its process number is someone else's after that.
    pub async fn save(&mut self) {
        match self.control {
            #[cfg(unix)]
            Control::Signals => {
                // SAFETY: a signal to our own child, which is still ours to wait for.
                unsafe { libc::kill(self.pid as i32, libc::SIGUSR1) };
            }
            Control::Stdin => self.say(SAVE).await,
        }
    }

    /// Ask it to stop. Whether it does is for the caller to wait and see.
    pub async fn stop(&mut self) {
        match self.control {
            #[cfg(unix)]
            Control::Signals => {
                // SAFETY: a signal to our own child, which has not been waited for yet.
                unsafe { libc::kill(self.pid as i32, libc::SIGINT) };
            }
            Control::Stdin => {
                self.say(STOP).await;
                // Closed as well: a recorder too busy to read the line
                // still sees its input end.
                self.stdin = None;
            }
        }
    }

    /// Write a line to the recorder. One that cannot be written means the
    /// recorder is going or gone, which its exit reports better than this.
    async fn say(&mut self, line: &str) {
        let Some(stdin) = &mut self.stdin else { return };
        let written = tokio::time::timeout(WRITE_WITHIN, async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            crate::warn(format_args!("zenithd: the recorder did not take \"{}\"", line.trim_end()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_control_lines_are_what_zenith_rec_reads() {
        assert_eq!((SAVE, STOP), ("save\n", "stop\n"));
    }

    #[test]
    fn only_stdin_is_asked_for_by_name() {
        assert_eq!(Control::named(Some("stdin")), Control::Stdin);
        assert_eq!(Control::named(None), Control::native());
        assert_eq!(Control::named(Some("signals")), Control::native());
        #[cfg(unix)]
        assert_eq!(Control::native(), Control::Signals);
    }

    /// A recorder that writes down what it is told, until its input ends.
    #[cfg(unix)]
    async fn told(control: Control, script: &str) -> (Link, Child) {
        let mut command = control.command("sh");
        let mut child = command.args(["-c", script]).stdout(Stdio::piped()).spawn().unwrap();
        let link = control.link(&mut child).unwrap();
        (link, child)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn save_and_stop_reach_a_recorder_that_reads_lines() {
        let (mut link, child) = told(Control::Stdin, "while IFS= read -r line; do echo \"got $line\"; done; echo closed").await;
        assert!(link.pid() > 0);
        link.save().await;
        link.save().await;
        link.stop().await;
        let out = child.wait_with_output().await.unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "got save\ngot save\ngot stop\nclosed\n", "the lines, then its input closing");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_recorder_that_is_gone_does_not_hold_anything_up() {
        let (mut link, mut child) = told(Control::Stdin, "exit 0").await;
        child.wait().await.unwrap();
        link.save().await;
        link.stop().await;
    }

    /// Whether the process `pid` is still running (not merely waiting to
    /// be collected).
    #[cfg(unix)]
    fn runs(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| stat.rsplit_once(')').is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z')))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_recorder_left_behind_is_stopped_and_nothing_else_is() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("zenith-clips.sock");
        let note = dir.path().join("zenith-clips.recorder");
        // Nothing written down: nothing to do.
        stop_leftover(&socket).await;

        // One that takes no notice of being asked, as a recorder that hangs does not.
        let mut left = std::process::Command::new("sh").args(["-c", "trap '' INT TERM; echo deaf; while :; do sleep 0.05; done"]).stdout(Stdio::piped()).spawn().unwrap();
        let mut said = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(left.stdout.take().unwrap()), &mut said).unwrap();
        assert_eq!(said, "deaf\n");
        let mut other = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        note_recorder(&socket, left.id());
        let written = std::fs::read_to_string(&note).unwrap();
        assert_eq!(noted(&written), Some(left.id()));

        // The same number with another start, or another boot, is not it:
        // that is what a number given out again looks like.
        let (_, rest) = written.split_once(' ').unwrap();
        let (started, boot) = rest.trim().split_once(' ').unwrap();
        for not_it in [format!("{} 1 {boot}", other.id()), format!("{} {started} another-boot", left.id()), format!("{} 1 {boot}", left.id()), "1 0 x".into(), "nonsense".into(), String::new()] {
            assert_eq!(noted(&not_it), None, "{not_it}");
            std::fs::write(&note, &not_it).unwrap();
            stop_leftover(&socket).await;
            assert!(runs(left.id()) && runs(other.id()), "{not_it}");
            assert!(!note.exists(), "a note is read once");
        }

        std::fs::write(&note, &written).unwrap();
        stop_leftover(&socket).await;
        assert!(!runs(left.id()), "the one written down is stopped, by force if it must be");
        assert!(runs(other.id()));
        left.wait().unwrap();
        other.kill().unwrap();
        other.wait().unwrap();

        note_recorder(&socket, std::process::id());
        forget_recorder(&socket);
        assert!(!note.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signals_reach_a_recorder_that_listens_for_them() {
        let script = "trap 'echo saved' USR1; trap 'echo stopped; exit 0' INT; echo ready; while :; do sleep 0.05 & wait $!; done";
        let (mut link, mut child) = told(Control::Signals, script).await;
        let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(child.stdout.take().unwrap()));
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("ready"));
        link.save().await;
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("saved"));
        link.stop().await;
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("stopped"));
        assert!(child.wait().await.unwrap().success());
    }
}
