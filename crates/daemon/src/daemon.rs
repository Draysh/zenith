//! The daemon: it keeps the recorder running when it should be, saves a clip
//! when asked, files it under the game being played, and says so. Every
//! decision about the recorder is the supervisor's (in the core crate); this
//! is the part that owns the process, the socket and the timers.

use crate::control::{Control, Link};
use crate::hotkey::Hotkeys;
use crate::{feedback, host, server, tray};
use zenith_core::recorder::outputs;
use zenith_core::config::{Activation, Config};
use zenith_core::games::{DESKTOP, Game, Scanner, folder_name};
use zenith_core::ipc::{Request, Response, Status};
use zenith_core::library::{self, Library, Meta};
use zenith_core::paths;
use zenith_core::recorder::supervisor::{Problem, save_timeout};
use zenith_core::recorder::{self, Action, Event, State, Supervisor};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{mpsc, oneshot};

/// How often the processes are looked through for a game.
const SCAN_EVERY: Duration = Duration::from_secs(3);
/// A recorder younger than this may not be listening for the save signal
/// yet, and a signal nobody listens for would end it.
const READY_AFTER: Duration = Duration::from_millis(1500);
/// How long the recorder gets to stop by itself before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// How long capture goes on after the game's last process is gone. A crash,
/// or a match that ends with the game closing, is often the moment to clip.
pub const GAME_GRACE: Duration = Duration::from_secs(30);
/// A recorder that would not stay up is tried again this often, for as long
/// as capture is wanted: what was wrong at login may have settled since.
const RETRY_FAILED_EVERY: Duration = Duration::from_secs(120);
/// How long ffprobe gets to read a clip that was just filed.
const PROBE_WITHIN: Duration = Duration::from_secs(30);
/// How long a clip that is being written or filed is waited for when the
/// daemon is told to stop. Less than the minute and a half a service is
/// given to stop in before it is killed.
const FINISH_WITHIN: Duration = Duration::from_secs(60);

pub struct Options {
    pub config: PathBuf,
    pub socket: PathBuf,
    pub index: PathBuf,
    /// The recorder's program: `gpu-screen-recorder` (`zenith-rec` on
    /// Windows), or a stand-in for tests.
    pub recorder: String,
    /// How that program is told to save and to stop.
    pub control: Control,
    /// How long a save may take, instead of what the buffer's size gives.
    pub save_timeout: Option<Duration>,
    /// How long capture goes on after the game is gone: `GAME_GRACE`.
    pub game_grace: Duration,
}

pub enum Msg {
    /// A line the recorder printed: the path of a clip it wrote.
    Line { generation: u64, line: String },
    Exited { generation: u64, ran: Duration, detail: String },
    /// The recorder said how it is recording.
    Way { generation: u64, way: String },
    Ipc(Request, oneshot::Sender<Response>),
    SaveTimeout(u64),
    RetryDue,
    /// Read the settings again, asked for by a signal. Windows has none:
    /// there it is only ever asked over the pipe.
    #[cfg_attr(windows, allow(dead_code))]
    Reload,
    Shutdown,
    /// A clip has been filed, or could not be.
    Filed(Filed),
    /// How taking the save hotkey went: the combination now held, or what
    /// is wrong with it, which the user is told. Windows holds one itself;
    /// on Linux the desktop gives one, and only ever says it is held.
    Hotkey(Result<String, String>),
    /// The desktop gives no save hotkey, and why (Linux). Not a fault:
    /// `zenithd save` bound to a key by hand works without one.
    #[cfg_attr(windows, allow(dead_code))]
    NoHotkey(String),
}

/// A clip the recorder wrote, after the work of filing it, which is done
/// aside: a clip folder on another disk makes it a copy of the whole file.
pub struct Filed {
    game: String,
    target: PathBuf,
    /// Why it could not be moved into the library; or, moved, what
    /// reading it gave.
    outcome: Result<Result<Meta, String>, String>,
    /// Who asked for this clip.
    waiting: Vec<oneshot::Sender<Response>>,
}

/// Move a clip the recorder wrote to its place in the library.
fn move_clip(written: &Path, target: &Path) -> std::io::Result<()> {
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder)?;
    }
    // A clip folder moved to another disk mid-save cannot be renamed into.
    std::fs::rename(written, target).or_else(|_| {
        // Copied is saved, whether or not the first can be cleared
        // away now (something may be reading it): that folder is
        // emptied when the recorder next starts.
        std::fs::copy(written, target).map(|_| {
            let _ = std::fs::remove_file(written);
        })
    })
}

/// Answer those waiting on a save.
fn tell(waiting: impl IntoIterator<Item = oneshot::Sender<Response>>, response: &Response) {
    for waiting in waiting {
        let _ = waiting.send(response.clone());
    }
}

/// What the task that owns the recorder's process is asked to do with it.
enum Order {
    Save,
    Stop,
}

struct Child {
    generation: u64,
    started: Instant,
    /// Dropped, it means stop as well: the daemon let go of the recorder.
    orders: mpsc::UnboundedSender<Order>,
    done: tokio::task::JoinHandle<()>,
}

struct Daemon {
    opts: Options,
    cfg: Config,
    sup: Supervisor,
    scanner: Scanner,
    game: Option<Game>,
    /// The game that just ended, and until when capture goes on for it.
    lingering: Option<(String, Instant)>,
    /// Since when the recorder has been given up on.
    failed_since: Option<Instant>,
    /// The last reason for giving up that the user was told, so the same
    /// one is not told again at every retry.
    reported: Option<String>,
    /// Roughly how much the running recorder's buffer holds.
    buffer_megabytes: u32,
    /// Where the running recorder writes, which new settings may move.
    incoming: Option<PathBuf>,
    /// Clips being filed now, and the folders of before that are cleared
    /// away once none is: one may still be copied out of them.
    filing: u32,
    stale: Vec<PathBuf>,
    /// The settings that were not valid when last read, as the user was told.
    ignored: Vec<String>,
    /// Capture switched on or off by hand, over what the settings say.
    forced: Option<bool>,
    child: Option<Child>,
    generation: u64,
    save_seq: u64,
    /// The game being played when the shortcut was pressed.
    save_game: Option<String>,
    waiting: Vec<oneshot::Sender<Response>>,
    /// Who asked it to quit, answered once it has cleaned up.
    quitting: Vec<oneshot::Sender<Response>>,
    library: Library,
    clips_saved: u32,
    hotkeys: Hotkeys,
    /// What is wrong with the save hotkey, while something is.
    hotkey_problem: Option<String>,
    /// The save hotkey that is held, and why none is where none is.
    hotkey_held: Option<String>,
    hotkey_missing: Option<String>,
    /// How the recorder said it is recording, for the one running now.
    way: Option<String>,
    tx: mpsc::UnboundedSender<Msg>,
}

fn log(text: impl std::fmt::Display) {
    crate::warn(format_args!("zenithd: {text}"));
}

/// Read lines until the other end closes. Bytes that are not text (a device
/// name in another encoding) are replaced, not a reason to stop reading.
async fn each_line(from: impl AsyncRead + Unpin, mut each: impl FnMut(String)) {
    let mut from = BufReader::new(from);
    let mut line = Vec::new();
    loop {
        line.clear();
        match from.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => each(String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']).to_string()),
        }
    }
}

/// Where the recorder writes before a clip is filed: inside the clip folder
/// (so filing is a rename), hidden from the library.
fn incoming(cfg: &Config) -> PathBuf {
    cfg.library.dir.join(".incoming")
}

impl Daemon {
    fn wanted(&self) -> bool {
        self.forced.unwrap_or(match self.cfg.capture.activation {
            Activation::Always => true,
            Activation::Game => self.game.is_some() || self.lingering.is_some(),
        })
    }

    /// The game a clip saved now belongs to: the one running, or the one
    /// that just ended.
    fn game_name(&self) -> Option<String> {
        self.game.as_ref().map(|g| g.name.clone()).or_else(|| self.lingering.as_ref().map(|(name, _)| name.clone()))
    }

    /// Take in what a look through the processes found.
    fn saw(&mut self, game: Option<Game>) {
        if game != self.game {
            log(match &game {
                Some(g) => format!("game running: {}", g.name),
                None => "no game running".into(),
            });
            self.lingering = match (&self.game, &game) {
                (Some(ended), None) => Some((ended.name.clone(), Instant::now() + self.opts.game_grace)),
                _ => None,
            };
            self.game = game;
        }
        if self.lingering.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            self.lingering = None;
        }
    }

    /// A recorder that was given up on is tried again after a while.
    async fn retry_failed(&mut self) {
        if self.wanted() && self.failed_since.is_some_and(|since| since.elapsed() >= RETRY_FAILED_EVERY) {
            log("trying the recorder again");
            self.failed_since = None;
            self.apply(Event::Want(true)).await;
        }
    }

    /// Keep track of how long the recorder has been given up on.
    fn note_state(&mut self) {
        let failed = matches!(self.sup.state(), State::Failed { .. });
        self.failed_since = if failed { self.failed_since.or_else(|| Some(Instant::now())) } else { None };
    }

    /// Give the supervisor an event and carry out what it decides, including
    /// what follows from that.
    async fn apply(&mut self, event: Event) {
        let mut queue: VecDeque<Action> = self.sup.handle(event).into();
        while let Some(action) = queue.pop_front() {
            if let Some(next) = self.act(action).await {
                queue.extend(self.sup.handle(next));
            }
        }
    }

    async fn sync_wanted(&mut self) {
        let wanted = self.wanted();
        if wanted != self.sup.wanted() {
            log(if wanted { "capture starts" } else { "capture stops" });
            self.apply(Event::Want(wanted)).await;
        }
    }

    async fn act(&mut self, action: Action) -> Option<Event> {
        match action {
            Action::Spawn => self.spawn().await.err().map(Event::SpawnFailed),
            Action::Kill => {
                self.stop_child().await;
                None
            }
            Action::SignalSave => {
                let Some(child) = &self.child else { return Some(Event::SaveTimedOut) };
                let (generation, age) = (child.generation, child.started.elapsed());
                if age < READY_AFTER {
                    tokio::time::sleep(READY_AFTER - age).await;
                }
                // It may have ended while that wait went on: its exit is on
                // its way here and says the clip was lost. The task that
                // owns the process passes the request on only while the
                // process is still its own.
                if let Some(child) = self.child.as_ref().filter(|c| c.generation == generation) {
                    let _ = child.orders.send(Order::Save);
                }
                self.save_seq += 1;
                let (tx, seq) = (self.tx.clone(), self.save_seq);
                let within = self.opts.save_timeout.unwrap_or_else(|| save_timeout(self.buffer_megabytes));
                tokio::spawn(async move {
                    tokio::time::sleep(within).await;
                    let _ = tx.send(Msg::SaveTimeout(seq));
                });
                None
            }
            Action::RetryIn(wait) => {
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let _ = tx.send(Msg::RetryDue);
                });
                None
            }
            Action::ClipSaved(path) => {
                self.file_clip(&path);
                None
            }
            Action::Report(Problem::RecorderUnsteady(why)) => {
                // Nobody is waiting on this, and it goes on being restarted.
                let detail = format!("The recorder stops after a short while, again and again. It is started again each time; clips may be missing. It last said: {why}");
                log(format!("Capture keeps stopping: {detail}"));
                if self.cfg.feedback.notification {
                    feedback::problem("Capture keeps stopping", &detail);
                }
                None
            }
            Action::Report(problem) => {
                let gave_up = matches!(problem, Problem::RecorderFailed(_));
                let (summary, detail) = match problem {
                    Problem::NotCapturing => (
                        "Nothing to clip",
                        match self.sup.state() {
                            State::Failed { reason } => format!("The recorder is not running: {reason}"),
                            State::Restarting { .. } => "The recorder stopped and is being restarted.".to_string(),
                            _ if self.forced == Some(false) => "Capture is switched off.".to_string(),
                            _ => "Capture runs while a game is running, and none was found.".to_string(),
                        },
                    ),
                    Problem::SaveFailed(why) => ("The clip was not saved", why),
                    Problem::RecorderFailed(why) | Problem::RecorderUnsteady(why) => ("Capture stopped", why),
                };
                log(format!("{summary}: {detail}"));
                self.answer(Response::Error { message: format!("{summary}: {detail}") });
                // Giving up for the same reason again, at a retry, is not news.
                let news = !gave_up || self.reported.as_deref() != Some(detail.as_str());
                if gave_up {
                    self.reported = Some(detail.clone());
                }
                if self.cfg.feedback.notification && news {
                    feedback::problem(summary, &detail);
                }
                None
            }
        }
    }

    /// Answer everyone waiting on a save.
    fn answer(&mut self, response: Response) {
        tell(self.waiting.drain(..), &response);
    }

    async fn spawn(&mut self) -> Result<(), String> {
        let dir = incoming(&self.cfg);
        // Anything left there is from a recorder that was killed mid-write;
        // so is the folder of before the clip folder was changed. Unless a
        // clip is being filed out of one of them right now: then they are
        // left until it has been.
        let old = self.incoming.replace(dir.clone()).filter(|old| *old != dir);
        if self.filing == 0 {
            if let Some(old) = old {
                let _ = std::fs::remove_dir_all(old);
            }
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            self.stale.extend(old);
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("the clip folder {} cannot be written to: {e}", dir.display()))?;

        // Asking the recorder what is plugged in runs it and waits for it,
        // which is done aside: requests are still taken and the timers
        // still run meanwhile. Nothing else is decided until it answers,
        // or has taken too long to (`outputs::ASK_WITHIN`).
        let recorder = self.opts.recorder.clone();
        let asked = tokio::task::spawn_blocking(move || (outputs::list(&recorder), outputs::audio_devices(&recorder))).await;
        let (monitors, devices) = asked.unwrap_or_default();
        let (source, native) = outputs::resolve(&self.cfg.capture.source, &monitors);
        let mut cfg = self.cfg.clone();
        cfg.capture.source = source;
        // Switched on by hand, there may be no game to wait for the
        // window of: the monitor is recorded from the start.
        if self.forced == Some(true) {
            cfg.capture.activation = Activation::Always;
        }
        // A chosen speaker or microphone that is not plugged in now gives
        // way to the default one, instead of the recorder refusing to start.
        for (chosen, default, what) in [
            (&mut cfg.audio.output_device, outputs::DEFAULT_OUTPUT, "output"),
            (&mut cfg.audio.input_device, outputs::DEFAULT_INPUT, "microphone"),
        ] {
            let now = outputs::present(chosen, default, &devices);
            if now != *chosen {
                log(format!("the chosen {what} ({chosen}) is not connected; recording the default one"));
                *chosen = now;
            }
        }
        let args = recorder::command_line(&cfg, native, &dir);
        self.buffer_megabytes = cfg.capture.buffer_megabytes(native, recorder::tracks(&cfg).len() as u32);
        let mut command = self.opts.control.command(&self.opts.recorder);
        command.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => format!("{} is not installed", self.opts.recorder),
                _ => format!("{} could not be started: {e}", self.opts.recorder),
            })?;
        let mut link: Link = self.opts.control.link(&mut child).ok_or("the recorder ended as it started")?;
        let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            return Err("the recorder's output could not be read".into());
        };
        self.generation += 1;
        let generation = self.generation;
        // gpu-screen-recorder does not say how it records; it is always the
        // graphics card's own way. Zenith's recorder says, a moment on.
        self.way = cfg!(not(windows)).then(|| format!("gpu-screen-recorder -> {}", cfg.capture.codec.as_str()));
        log(format!("recorder started (pid {}): {} {}", link.pid(), self.opts.recorder, args.join(" ")));
        crate::control::note_recorder(&self.opts.socket, link.pid());

        let tx = self.tx.clone();
        let lines = tokio::spawn(each_line(stdout, move |line| {
            let _ = tx.send(Msg::Line { generation, line });
        }));
        // The last thing it said is the best account of why it stopped.
        let last_words = Arc::new(Mutex::new(String::new()));
        let words = last_words.clone();
        let tx = self.tx.clone();
        let errors = tokio::spawn(each_line(stderr, move |line| {
            if !line.trim().is_empty() {
                *words.lock().unwrap_or_else(|e| e.into_inner()) = line.trim().to_string();
            }
            if let Some(way) = recorder::way_from_line(&line) {
                let _ = tx.send(Msg::Way { generation, way });
            }
        }));

        let (orders, mut ordered) = mpsc::unbounded_channel();
        let tx = self.tx.clone();
        let started = Instant::now();
        let done = tokio::spawn(async move {
            loop {
                tokio::select! {
                    status = child.wait() => {
                        // What it printed last is read by other tasks, and
                        // must be handed on before the news that it ended:
                        // a recorder may print a clip's path and end at
                        // once. A helper of its that lives on may hold the
                        // pipes open: what it said so far has to do.
                        let _ = tokio::time::timeout(Duration::from_millis(300), async move {
                            let _ = lines.await;
                            let _ = errors.await;
                        })
                        .await;
                        let said = last_words.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        let status = status.map_or_else(|e| e.to_string(), |s| s.to_string());
                        let detail = if said.is_empty() { status } else { format!("{said} ({status})") };
                        let _ = tx.send(Msg::Exited { generation, ran: started.elapsed(), detail });
                        return;
                    }
                    order = ordered.recv() => match order {
                        Some(Order::Save) => link.save().await,
                        // Asked to stop, or the daemon let go of it.
                        Some(Order::Stop) | None => break,
                    },
                }
            }
            link.stop().await;
            if tokio::time::timeout(STOP_GRACE, child.wait()).await.is_err() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        });
        self.child = Some(Child { generation, started, orders, done });
        Ok(())
    }

    /// Stop the recorder and wait until it is gone, so two never run at once.
    async fn stop_child(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.orders.send(Order::Stop);
            let _ = child.done.await;
            crate::control::forget_recorder(&self.opts.socket);
            log("recorder stopped");
        }
    }

    /// File a clip the recorder wrote: under its game, into the library.
    /// The moving and the reading are done aside and come back as
    /// `Msg::Filed`; whoever asked for the clip is answered then.
    fn file_clip(&mut self, written: &Path) {
        let game = self.save_game.take().or_else(|| self.game_name()).unwrap_or_else(|| DESKTOP.into());
        let target = library::clip_path(&self.cfg.library.dir, &game, chrono::Local::now());
        let waiting = std::mem::take(&mut self.waiting);
        // Within the clip folder, as nearly always, filing is a rename:
        // done here and now, so that the name is taken before the next
        // clip is given one.
        let renamed = target.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::rename(written, &target)).is_ok();
        self.filing += 1;
        let (tx, written) = (self.tx.clone(), written.to_path_buf());
        tokio::spawn(async move {
            let to = target.clone();
            let moved = if renamed { Ok(()) } else { tokio::task::spawn_blocking(move || move_clip(&written, &to)).await.unwrap_or_else(|e| Err(std::io::Error::other(e))) };
            let outcome = match moved {
                Err(e) => Err(format!("it could not be moved to {}: {e}", target.display())),
                Ok(()) => {
                    let file = target.clone();
                    let read = tokio::time::timeout(PROBE_WITHIN, tokio::task::spawn_blocking(move || library::probe(&file).map_err(|e| e.to_string()))).await;
                    Ok(match read {
                        Ok(Ok(read)) => read,
                        Ok(Err(e)) => Err(e.to_string()),
                        Err(_) => Err("ffprobe did not answer in time".to_string()),
                    })
                }
            };
            let _ = tx.send(Msg::Filed(Filed { game, target, outcome, waiting }));
        });
    }

    /// Take in a clip that was filed: index it, and say so.
    fn filed(&mut self, Filed { game, target, outcome, waiting }: Filed) {
        self.filing = self.filing.saturating_sub(1);
        if self.filing == 0 {
            for old in std::mem::take(&mut self.stale) {
                if self.incoming.as_ref() != Some(&old) {
                    let _ = std::fs::remove_dir_all(old);
                }
            }
        }
        let read = match outcome {
            Ok(read) => read,
            Err(why) => {
                log(format!("the clip was not saved: {why}"));
                tell(waiting, &Response::Error { message: format!("The clip was not saved: {why}") });
                if self.cfg.feedback.notification {
                    feedback::problem("The clip was not saved", &why);
                }
                return;
            }
        };
        // From here on the file is in the library's folder: the clip is
        // saved, whatever else goes wrong.
        match &read {
            Ok(meta) => {
                let tracks: Vec<_> = recorder::tracks(&self.cfg).into_iter().map(|t| t.0).collect();
                let saved_at = library::saved_at(&target);
                if let Err(e) = self.library.add(&target, &folder_name(&game), saved_at, meta, &tracks) {
                    // The file is safe; the index is rebuilt from the folder.
                    log(format!("the clip is saved but not indexed: {e}"));
                }
                log(format!("clip saved: {} ({:.1} s)", target.display(), meta.seconds));
            }
            // Without ffprobe, or with a file it will not read, there is
            // nothing to index it with. The app looks through the folder
            // when it opens and takes the clip in once it can be read.
            Err(why) => log(format!("clip saved: {}; it could not be read, so the library takes it in later: {why}", target.display())),
        }
        self.clips_saved += 1;
        self.reported = None;
        tell(waiting, &Response::Saved { path: target.clone() });
        if self.cfg.feedback.sound {
            feedback::play_saved();
        }
        if self.cfg.feedback.notification {
            feedback::clip_saved(&game, read.as_ref().ok().map(|meta| meta.seconds), &target);
        }
    }

    /// Whether a clip is on its way: being written by the recorder, or
    /// being filed.
    fn saving(&self) -> bool {
        *self.sup.state() == State::Saving || self.filing > 0
    }

    /// Told to stop while a clip is on its way: wait for it, for a while,
    /// rather than throw it away. Nothing new is started meanwhile.
    async fn finish_saves(&mut self, rx: &mut mpsc::UnboundedReceiver<Msg>) {
        if !self.saving() {
            return;
        }
        log("a clip is being saved; stopping once it is");
        // No longer wanted: the recorder is stopped when its clip is out,
        // not started afresh.
        self.forced = Some(false);
        self.apply(Event::Want(false)).await;
        let until = tokio::time::Instant::now() + FINISH_WITHIN;
        while self.saving() {
            let Ok(Some(msg)) = tokio::time::timeout_at(until, rx.recv()).await else {
                log("the clip took too long; stopping without it");
                break;
            };
            match msg {
                Msg::Ipc(Request::Quit, reply) => self.quitting.push(reply),
                Msg::Ipc(Request::Status, reply) => {
                    let _ = reply.send(Response::Status(self.status()));
                }
                Msg::Ipc(_, reply) => {
                    let _ = reply.send(Response::Error { message: "the daemon is shutting down".into() });
                }
                // Told a second time: now.
                Msg::Shutdown => break,
                Msg::Reload | Msg::RetryDue | Msg::Hotkey(_) | Msg::NoHotkey(_) => {}
                // What the recorder says and does, the save's timer, the filing.
                about_the_clip => {
                    self.on(about_the_clip).await;
                }
            }
        }
    }

    fn status(&self) -> Status {
        let (state, detail) = match self.sup.state() {
            State::Idle => ("idle", None),
            State::Capturing => ("capturing", None),
            State::Saving => ("saving", None),
            State::Restarting { attempt } => ("restarting", Some(format!("attempt {attempt}"))),
            State::Failed { reason } => ("failed", Some(reason.clone())),
        };
        Status {
            state: state.into(),
            // With nothing to say about the recorder, a hotkey that does
            // not work is what there is to know.
            detail: detail.or_else(|| self.hotkey_problem.clone()),
            game: self.game.as_ref().map(|g| g.name.clone()),
            forced: self.forced,
            buffer_secs: self.cfg.capture.buffer_secs,
            clips_saved: self.clips_saved,
            recording: self.way.clone().filter(|_| matches!(self.sup.state(), State::Capturing | State::Saving)),
            version: env!("CARGO_PKG_VERSION").into(),
            hotkey: self.hotkey_held.clone(),
            hotkey_missing: self.hotkey_missing.clone(),
        }
    }

    /// Take in which settings were not valid, and say so when it is news.
    fn note_ignored(&mut self, ignored: Vec<String>, notify: bool) {
        if ignored != self.ignored && !ignored.is_empty() {
            // The rest of the settings count; these are at their defaults.
            let names = ignored.join(", ");
            log(format!("settings that are not valid were ignored: {names}"));
            if notify {
                feedback::problem("Some of Zenith's settings are not valid", &format!("Ignored, until they are fixed: {names}"));
            }
        }
        self.ignored = ignored;
    }

    /// Read the settings again, the way they are read at the start: one
    /// value that is not valid is left at its default and named, and does
    /// not keep the rest from counting. Only a file that cannot be read at
    /// all is refused, and the settings stay as they were.
    async fn reload(&mut self) -> Result<(), String> {
        let (cfg, ignored) = Config::load_lenient(&self.opts.config).map_err(|e| e.to_string())?;
        self.note_ignored(ignored, cfg.feedback.notification);
        // A new combination, or another try at one that could not be taken.
        if cfg.capture.hotkey != self.cfg.capture.hotkey || self.hotkey_problem.is_some() {
            self.hotkeys.set(&cfg.capture.hotkey);
        }
        if cfg != self.cfg {
            let restart = !cfg.capture.records_like(&self.cfg.capture) || cfg.audio != self.cfg.audio || cfg.library != self.cfg.library;
            self.cfg = cfg;
            log("settings reloaded");
            if restart {
                self.apply(Event::Reconfigure).await;
            }
            self.sync_wanted().await;
        }
        Ok(())
    }

    /// Returns whether to keep running.
    async fn on(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Line { generation, line } => {
                let path = PathBuf::from(line.trim());
                let ours = self.child.as_ref().is_some_and(|c| c.generation == generation);
                if ours && path.is_absolute() && path.is_file() {
                    self.apply(Event::Saved(path)).await;
                } else if ours && let Some(why) = recorder::save_failure_from_line(&line) {
                    // Said at once, and the recorder left running: waiting
                    // for the save's time to run out would end it too.
                    log(format!("the recorder could not save the clip: {why}"));
                    self.apply(Event::SaveFailed(why)).await;
                }
            }
            Msg::Exited { generation, ran, detail } => {
                if self.child.as_ref().is_some_and(|c| c.generation == generation) {
                    self.child = None;
                    crate::control::forget_recorder(&self.opts.socket);
                    log(format!("recorder ended after {:.1} s: {detail}", ran.as_secs_f64()));
                    self.apply(Event::Exited { ran, detail }).await;
                }
            }
            Msg::Way { generation, way } => {
                if generation == self.generation {
                    if recorder::is_fallback(&way) {
                        log(format!("recording the slower way: {way}"));
                    }
                    self.way = Some(way);
                }
            }
            Msg::SaveTimeout(seq) => {
                if seq == self.save_seq && *self.sup.state() == State::Saving {
                    self.apply(Event::SaveTimedOut).await;
                }
            }
            Msg::RetryDue => self.apply(Event::RetryDue).await,
            Msg::Reload => {
                if let Err(e) = self.reload().await {
                    log(format!("the settings were not reloaded: {e}"));
                }
            }
            Msg::Ipc(request, reply) => match request {
                Request::Save => {
                    self.save_game = self.game_name();
                    self.waiting.push(reply);
                    self.apply(Event::SaveRequested).await;
                    // This clip is lost, but the press says capture is
                    // missed: a recorder that was given up on is tried again.
                    if self.wanted() && matches!(self.sup.state(), State::Failed { .. }) {
                        self.failed_since = None;
                        self.apply(Event::Want(true)).await;
                    }
                }
                Request::Status => {
                    let _ = reply.send(Response::Status(self.status()));
                }
                Request::Capture { force } => {
                    self.forced = force;
                    self.reported = None;
                    if force == Some(true) {
                        // Asked for by hand: also a way to start a recorder
                        // that was given up on.
                        self.apply(Event::Want(true)).await;
                    } else {
                        self.sync_wanted().await;
                    }
                    let _ = reply.send(Response::Ok);
                }
                Request::Reload => {
                    self.reported = None;
                    let answer = self.reload().await.map_or_else(|message| Response::Error { message }, |_| Response::Ok);
                    let _ = reply.send(answer);
                }
                Request::Quit => {
                    self.quitting.push(reply);
                    return false;
                }
            },
            Msg::Shutdown => return false,
            Msg::Filed(filed) => self.filed(filed),
            Msg::Hotkey(outcome) => self.hotkey_went(outcome),
            Msg::NoHotkey(why) => {
                // One plain line, and not the same one twice running.
                if self.hotkey_missing.as_deref() != Some(why.as_str()) {
                    log(format!("no save hotkey from the desktop: {why}. `zenithd save` bound to a key in the desktop's settings works as before"));
                }
                self.hotkey_held = None;
                self.hotkey_missing = Some(why);
            }
        }
        true
    }

    /// Take in how holding the save hotkey went, and say so when it is
    /// news: a hotkey that silently does nothing is the worst of it.
    fn hotkey_went(&mut self, outcome: Result<String, String>) {
        self.hotkey_missing = None;
        self.hotkey_held = outcome.as_ref().ok().cloned();
        let problem = match outcome {
            Ok(held) => {
                log(format!("the save hotkey is {held}"));
                None
            }
            Err(why) => {
                log(&why);
                if self.cfg.feedback.notification && self.hotkey_problem.as_deref() != Some(why.as_str()) {
                    feedback::problem("The save hotkey does not work", &why);
                }
                Some(why)
            }
        };
        self.hotkey_problem = problem;
    }
}

pub async fn run(opts: Options) -> Result<(), String> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    // Listening is also what makes this the only daemon. Requests that
    // come in from here on wait in the channel until the loop below runs.
    let _only: server::Only = server::listen(&opts.socket, tx.clone())?;
    // Windows has no journal to keep what a background program says. Only
    // now: a second daemon, which ends above, must not put aside the log
    // the first is writing.
    if cfg!(windows)
        && let Err(e) = crate::logfile::start(&paths::data_dir().join("logs").join("zenithd.log"))
    {
        crate::warn(format_args!("zenithd: no log file is kept: {e}"));
    }

    // A daemon that was killed may have left its recorder running, and
    // that one would record on beside the one started here.
    crate::control::stop_leftover(&opts.socket).await;

    let (cfg, ignored) = match Config::load_lenient(&opts.config) {
        Ok(read) => read,
        Err(e) => {
            // Clipping with the defaults beats not clipping.
            log(format!("{e}; the defaults are used"));
            feedback::problem("Zenith's settings are not valid", &e.to_string());
            (Config::default(), Vec::new())
        }
    };
    let library = Library::open(&opts.index).map_err(|e| e.to_string())?;
    let home = if cfg!(windows) { paths::home() } else { std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default() };
    let mut daemon = Daemon {
        cfg,
        sup: Supervisor::default(),
        scanner: Scanner::new(&home),
        game: None,
        lingering: None,
        failed_since: None,
        reported: None,
        buffer_megabytes: 0,
        incoming: None,
        filing: 0,
        stale: Vec::new(),
        ignored: Vec::new(),
        forced: None,
        child: None,
        generation: 0,
        save_seq: 0,
        save_game: None,
        waiting: Vec::new(),
        quitting: Vec::new(),
        library,
        clips_saved: 0,
        hotkeys: Hotkeys::start(tx.clone()),
        hotkey_problem: None,
        hotkey_held: None,
        hotkey_missing: None,
        way: None,
        tx: tx.clone(),
        opts,
    };
    daemon.note_ignored(ignored, true);
    daemon.hotkeys.set(&daemon.cfg.capture.hotkey);
    host::signals(&tx)?;

    let no_tray = std::env::var_os("ZENITH_NO_TRAY").is_some();
    let icon = if no_tray { None } else { tray::start(daemon.status(), daemon.cfg.library.dir.clone(), tx.clone()).await };
    let mut shown = (daemon.status(), daemon.cfg.library.dir.clone());
    let mut asleep = host::slept();
    log(format!("ready; clips go to {}", daemon.cfg.library.dir.display()));
    let mut scan = tokio::time::interval(SCAN_EVERY);
    scan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = scan.tick() => {
                let game = daemon.scanner.scan(&daemon.cfg.games.added);
                daemon.saw(game);
                daemon.sync_wanted().await;
                daemon.retry_failed().await;
                // Awake again: the monitors and the sound may not be what
                // the recorder was started with.
                let now = host::slept();
                if now > asleep + Duration::from_secs(1) {
                    log("the machine woke up");
                    daemon.apply(Event::Refresh).await;
                }
                asleep = now;
            }
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                if !daemon.on(msg).await {
                    break;
                }
            }
        }
        daemon.note_state();
        let now = (daemon.status(), daemon.cfg.library.dir.clone());
        if now != shown {
            tray::show(&icon, now.0.clone(), &now.1).await;
            shown = now;
        }
    }
    tray::stop(icon).await;

    daemon.finish_saves(&mut rx).await;
    daemon.stop_child().await;
    daemon.answer(Response::Error { message: "the daemon is shutting down".into() });
    // A clip still being copied out of it is not pulled from under the copy.
    if daemon.filing == 0 {
        let _ = std::fs::remove_dir_all(incoming(&daemon.cfg));
    }
    server::close(&daemon.opts.socket);
    tell(daemon.quitting.drain(..), &Response::Ok);
    // Let the answers still being written go out before the runtime ends.
    tokio::time::sleep(Duration::from_millis(50)).await;
    log("stopped");
    Ok(())
}
