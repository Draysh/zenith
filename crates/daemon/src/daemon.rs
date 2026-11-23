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
use zenith_core::library::{self, Library};
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
    /// How taking the save hotkey went: the combination now held, or what
    /// is wrong with it. Only Windows has the daemon hold one.
    #[cfg_attr(not(windows), allow(dead_code))]
    Hotkey(Result<String, String>),
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
    /// Capture switched on or off by hand, over what the settings say.
    forced: Option<bool>,
    child: Option<Child>,
    generation: u64,
    save_seq: u64,
    /// The game being played when the shortcut was pressed.
    save_game: Option<String>,
    waiting: Vec<oneshot::Sender<Response>>,
    /// Who asked it to quit, answered once it has cleaned up.
    quitting: Option<oneshot::Sender<Response>>,
    library: Library,
    clips_saved: u32,
    hotkeys: Hotkeys,
    /// What is wrong with the save hotkey, while something is.
    hotkey_problem: Option<String>,
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
            Action::Spawn => self.spawn().err().map(Event::SpawnFailed),
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
                    Problem::RecorderFailed(why) => ("Capture stopped", why),
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
        for waiting in self.waiting.drain(..) {
            let _ = waiting.send(response.clone());
        }
    }

    fn spawn(&mut self) -> Result<(), String> {
        let dir = incoming(&self.cfg);
        // Anything left there is from a recorder that was killed mid-write;
        // so is the folder of before the clip folder was changed.
        if let Some(old) = self.incoming.replace(dir.clone()).filter(|old| *old != dir) {
            let _ = std::fs::remove_dir_all(old);
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("the clip folder {} cannot be written to: {e}", dir.display()))?;

        let (source, native) = outputs::resolve(&self.cfg.capture.source, &outputs::list(&self.opts.recorder));
        let mut cfg = self.cfg.clone();
        cfg.capture.source = source;
        // Switched on by hand, there may be no game to wait for the
        // window of: the monitor is recorded from the start.
        if self.forced == Some(true) {
            cfg.capture.activation = Activation::Always;
        }
        // A chosen speaker or microphone that is not plugged in now gives
        // way to the default one, instead of the recorder refusing to start.
        let devices = outputs::audio_devices(&self.opts.recorder);
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

        let tx = self.tx.clone();
        tokio::spawn(each_line(stdout, move |line| {
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
                        // A helper of its that lives on may hold the pipe open:
                        // what it said so far has to do.
                        let _ = tokio::time::timeout(Duration::from_millis(300), errors).await;
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
            log("recorder stopped");
        }
    }

    /// File a clip the recorder wrote: under its game, into the library.
    fn file_clip(&mut self, written: &Path) {
        let game = self.save_game.take().or_else(|| self.game_name()).unwrap_or_else(|| DESKTOP.into());
        let target = library::clip_path(&self.cfg.library.dir, &game, chrono::Local::now());
        let filed = target
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            // A clip folder moved to another disk mid-save cannot be renamed into.
            .and_then(|_| std::fs::rename(written, &target).or_else(|_| {
                // Copied is saved, whether or not the first can be cleared
                // away now (something may be reading it): that folder is
                // emptied when the recorder next starts.
                std::fs::copy(written, &target).map(|_| {
                    let _ = std::fs::remove_file(written);
                })
            }))
            .map_err(|e| format!("it could not be moved to {}: {e}", target.display()))
            .and_then(|_| library::probe(&target).map_err(|e| format!("{} was written but cannot be played: {e}", target.display())));
        match filed {
            Ok(meta) => {
                let tracks: Vec<_> = recorder::tracks(&self.cfg).into_iter().map(|t| t.0).collect();
                let saved_at = library::saved_at(&target);
                if let Err(e) = self.library.add(&target, &folder_name(&game), saved_at, &meta, &tracks) {
                    // The file is safe; the index is rebuilt from the folder.
                    log(format!("the clip is saved but not indexed: {e}"));
                }
                self.clips_saved += 1;
                self.reported = None;
                log(format!("clip saved: {} ({:.1} s)", target.display(), meta.seconds));
                self.answer(Response::Saved { path: target.clone() });
                if self.cfg.feedback.sound {
                    feedback::play_saved();
                }
                if self.cfg.feedback.notification {
                    feedback::clip_saved(&game, meta.seconds, &target);
                }
            }
            Err(why) => {
                log(format!("the clip was not saved: {why}"));
                self.answer(Response::Error { message: format!("The clip was not saved: {why}") });
                if self.cfg.feedback.notification {
                    feedback::problem("The clip was not saved", &why);
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
        }
    }

    async fn reload(&mut self) -> Result<(), String> {
        let cfg = Config::load(&self.opts.config).map_err(|e| e.to_string())?;
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
                if self.child.as_ref().is_some_and(|c| c.generation == generation) && path.is_absolute() && path.is_file() {
                    self.apply(Event::Saved(path)).await;
                }
            }
            Msg::Exited { generation, ran, detail } => {
                if self.child.as_ref().is_some_and(|c| c.generation == generation) {
                    self.child = None;
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
                    self.quitting = Some(reply);
                    return false;
                }
            },
            Msg::Shutdown => return false,
            Msg::Hotkey(outcome) => self.hotkey_went(outcome),
        }
        true
    }

    /// Take in how holding the save hotkey went, and say so when it is
    /// news: a hotkey that silently does nothing is the worst of it.
    fn hotkey_went(&mut self, outcome: Result<String, String>) {
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

    let cfg = match Config::load_lenient(&opts.config) {
        Ok((cfg, ignored)) => {
            if !ignored.is_empty() {
                // The rest of the settings count; these are at their defaults.
                let names = ignored.join(", ");
                log(format!("settings that are not valid were ignored: {names}"));
                feedback::problem("Some of Zenith's settings are not valid", &format!("Ignored, until they are fixed: {names}"));
            }
            cfg
        }
        Err(e) => {
            // Clipping with the defaults beats not clipping.
            log(format!("{e}; the defaults are used"));
            feedback::problem("Zenith's settings are not valid", &e.to_string());
            Config::default()
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
        forced: None,
        child: None,
        generation: 0,
        save_seq: 0,
        save_game: None,
        waiting: Vec::new(),
        quitting: None,
        library,
        clips_saved: 0,
        hotkeys: Hotkeys::start(tx.clone()),
        hotkey_problem: None,
        way: None,
        tx: tx.clone(),
        opts,
    };
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

    daemon.stop_child().await;
    daemon.answer(Response::Error { message: "the daemon is shutting down".into() });
    let _ = std::fs::remove_dir_all(incoming(&daemon.cfg));
    server::close(&daemon.opts.socket);
    if let Some(reply) = daemon.quitting.take() {
        let _ = reply.send(Response::Ok);
    }
    // Let the answers still being written go out before the runtime ends.
    tokio::time::sleep(Duration::from_millis(50)).await;
    log("stopped");
    Ok(())
}
