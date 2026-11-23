//! Where each track's sound comes from. What an `-a` value means, which
//! programs' sound a track takes and what to do when a device will not open
//! is decided here, in plain logic; opening the devices themselves is the
//! small Windows part, behind [`Opener`].

pub mod processes;
#[cfg(windows)]
mod windows;

use crate::mixer::{TrackBuffer, TrackStats};
use processes::{Proc, ProcessTable};
use std::sync::Arc;
use std::time::Duration;

/// What a track records, read from its `-a` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spec {
    /// Everything played on the default speakers.
    DefaultOutput,
    /// The default microphone.
    DefaultInput,
    /// Everything played on the speakers with this id.
    Output(String),
    /// The microphone with this id.
    Input(String),
    /// Only these programs.
    Apps(Vec<String>),
    /// Everything except these programs.
    AllExcept(Vec<String>),
    /// Several of the above, added together.
    Several(Vec<Spec>),
}

/// The ending that marks a device name as "what plays on these speakers",
/// which is how the Linux recorder names them and how the app tells
/// speakers from microphones.
pub const MONITOR_SUFFIX: &str = ".monitor";

fn parse_one(item: &str) -> Spec {
    match item {
        "default_output" => Spec::DefaultOutput,
        "default_input" => Spec::DefaultInput,
        _ => match item.strip_suffix(MONITOR_SUFFIX) {
            Some(id) => Spec::Output(id.to_string()),
            None => Spec::Input(item.to_string()),
        },
    }
}

/// Read an `-a` value: `default_output`, `default_input`, a device's name,
/// `app:A|app:B`, `app-inverse:A|app-inverse:B`, or devices joined with `|`.
pub fn parse_spec(value: &str) -> Spec {
    let items: Vec<&str> = value.split('|').map(str::trim).filter(|i| !i.is_empty()).collect();
    let all_with = |prefix: &str| -> Option<Vec<String>> { items.iter().map(|i| i.strip_prefix(prefix).map(String::from)).collect() };
    if items.is_empty() {
        // Nothing asked for: a silent track still keeps its place.
        return Spec::Several(Vec::new());
    }
    if let Some(apps) = all_with("app-inverse:") {
        return Spec::AllExcept(apps);
    }
    if let Some(apps) = all_with("app:") {
        return Spec::Apps(apps);
    }
    let mut parts: Vec<Spec> = items
        .iter()
        .map(|item| match (item.strip_prefix("app:"), item.strip_prefix("app-inverse:")) {
            (Some(app), _) => Spec::Apps(vec![app.to_string()]),
            (_, Some(app)) => Spec::AllExcept(vec![app.to_string()]),
            _ => parse_one(item),
        })
        .collect();
    if parts.len() == 1 { parts.remove(0) } else { Spec::Several(parts) }
}

impl Spec {
    /// Whether the running programs have to be known to record this.
    pub fn needs_processes(&self) -> bool {
        match self {
            Spec::Apps(_) | Spec::AllExcept(_) => true,
            Spec::Several(parts) => parts.iter().any(Spec::needs_processes),
            _ => false,
        }
    }
}

/// One stream of sound to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// What plays on some speakers; `None` is the default ones.
    Output(Option<String>),
    /// A microphone; `None` is the default one.
    Input(Option<String>),
    /// The sound of the program `pid` and all it started (`include`), or
    /// of everything but those.
    Tree { pid: u32, include: bool },
}

/// The streams a track is made of right now, and anything worth a log line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub taps: Vec<Tap>,
    pub notes: Vec<String>,
}

/// The streams that record `spec` while `procs` are the programs running.
///
/// Windows can leave out one program (with all it started) from a recording
/// of everything, not several: with more than one voice program running,
/// the first named is left out and the others stay in the game track.
pub fn plan(spec: &Spec, procs: &[Proc]) -> Plan {
    let mut out = Plan::default();
    match spec {
        Spec::DefaultOutput => out.taps.push(Tap::Output(None)),
        Spec::DefaultInput => out.taps.push(Tap::Input(None)),
        Spec::Output(id) => out.taps.push(Tap::Output(Some(id.clone()))),
        Spec::Input(id) => out.taps.push(Tap::Input(Some(id.clone()))),
        Spec::Apps(apps) => {
            out.taps.extend(processes::roots(apps, procs).into_iter().map(|root| Tap::Tree { pid: root.pid, include: true }));
        }
        Spec::AllExcept(apps) => {
            let roots = processes::roots(apps, procs);
            match roots.first() {
                None => out.taps.push(Tap::Output(None)),
                Some(first) => {
                    out.taps.push(Tap::Tree { pid: first.pid, include: false });
                    if roots.len() > 1 {
                        let others: Vec<String> = roots[1..].iter().map(|r| format!("{} ({})", r.name, r.pid)).collect();
                        out.notes.push(format!(
                            "only one program can be left out of a recording: {} ({}) is, {} will be heard in this track too",
                            first.name,
                            first.pid,
                            others.join(", ")
                        ));
                    }
                }
            }
        }
        Spec::Several(parts) => {
            for part in parts {
                let sub = plan(part, procs);
                out.taps.extend(sub.taps);
                out.notes.extend(sub.notes);
            }
        }
    }
    out
}

/// An open stream of sound.
pub trait Stream {
    /// Append what was captured since the last call: 48 kHz stereo floats,
    /// interleaved. An error means the stream is finished (its device was
    /// unplugged, its program ended) and should be opened again.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), String>;
}

/// Opens streams. The Windows one is the only real one.
pub trait Opener {
    fn open(&mut self, tap: &Tap) -> Result<Box<dyn Stream>, String>;
    /// The id of the default speakers (`output`) or microphone, so that a
    /// change of default is noticed. `None` when there is none.
    fn default_id(&mut self, output: bool) -> Option<String>;
}

/// What the mixer takes a track's sound from.
pub trait Source {
    /// Fill `out` with the track's next frames (48 kHz stereo floats,
    /// interleaved), silence where it has none. `now` is the time since
    /// the mixer started; `owed` is how many frames the clock will ask for
    /// straight after these, to catch up on a pause. Called at every
    /// wake-up, with nothing to fill when nothing is due. This must never
    /// fail and never wait.
    fn fill(&mut self, now: Duration, out: &mut [f32], owed: usize);
    /// What became of the track's sound so far.
    fn stats(&self) -> TrackStats;
    /// A few words for the log about how the source is doing.
    fn status(&self) -> String;
}

/// How often the programs and default devices are looked at again.
pub const RESCAN: Duration = Duration::from_secs(3);
/// The longest a stream that will not open is left before the next try.
pub const MAX_RETRY: Duration = Duration::from_secs(60);

/// How long a stream that would not open `failures` times in a row is
/// left before the next try: a look's time at first, twice as long each
/// time after, up to a minute. A try can take its time (a headset that is
/// switched off is waited for), and it is made on the thread that mixes.
pub fn retry_delay(failures: u32) -> Duration {
    (RESCAN * 2u32.pow(failures.clamp(1, 8) - 1)).min(MAX_RETRY)
}

struct Slot {
    tap: Tap,
    /// The default device the tap stood for when it was opened.
    default_id: Option<String>,
    stream: Option<Box<dyn Stream>>,
    /// The last failure said in the log, so it is said once.
    said: Option<String>,
    /// How many times in a row it would not open, and when (by the
    /// mixer's time) it is tried next.
    failures: u32,
    retry_at: Duration,
    /// What the stream gave and the clock has not taken yet. Each stream
    /// has its own: they give their sound a packet at a time, each at its
    /// own moment, and are only in step once each is laid against the clock.
    buffer: TrackBuffer,
}

/// A track's source: its plan carried out with an [`Opener`], looked at
/// again every few seconds, its streams added together.
pub struct TrackSource {
    label: String,
    spec: Spec,
    opener: Box<dyn Opener>,
    table: Arc<ProcessTable>,
    slots: Vec<Slot>,
    planned: Option<Plan>,
    next_scan: Duration,
    scratch: Vec<f32>,
    one: Vec<f32>,
    /// The counts of streams that are no longer part of the track, and
    /// the silence of when it had none.
    past: TrackStats,
    recreations: u32,
    last_error: Option<String>,
}

impl TrackSource {
    pub fn new(label: String, spec: Spec, opener: Box<dyn Opener>, table: Arc<ProcessTable>) -> Self {
        Self {
            label,
            spec,
            opener,
            table,
            slots: Vec::new(),
            planned: None,
            next_scan: Duration::ZERO,
            scratch: Vec::new(),
            one: Vec::new(),
            past: TrackStats::default(),
            recreations: 0,
            last_error: None,
        }
    }

    fn default_of(&mut self, tap: &Tap) -> Option<String> {
        match tap {
            // Everything but one program may be standing in as everything
            // on the default speakers (see `open`), which has to follow
            // them like any other; opening it again also tries once more
            // to leave the program out.
            Tap::Output(None) | Tap::Tree { include: false, .. } => self.opener.default_id(true),
            Tap::Input(None) => self.opener.default_id(false),
            _ => None,
        }
    }

    /// Open `tap`. Leaving a program out is the one thing with something to
    /// fall back on: everything that plays, voices included, rather than
    /// a silent game track.
    fn open(&mut self, tap: &Tap, said: &mut Option<String>) -> Option<Box<dyn Stream>> {
        let mut result = self.opener.open(tap);
        let mut fell_back = false;
        if let (Err(err), Tap::Tree { include: false, pid }) = (&result, tap) {
            let message = format!("could not leave program {pid} out ({err}); recording everything that plays instead");
            self.say(said, message);
            result = self.opener.open(&Tap::Output(None));
            fell_back = true;
        }
        match result {
            Ok(stream) => {
                // After a fallback the complaint stands: it is still true.
                if !fell_back && said.take().is_some() {
                    crate::note!("{}: {tap:?} is open again", self.label);
                }
                Some(stream)
            }
            Err(err) => {
                self.say(said, format!("{tap:?} could not be opened ({err}); silent until it can"));
                None
            }
        }
    }

    fn say(&mut self, said: &mut Option<String>, message: String) {
        if said.as_ref() != Some(&message) {
            crate::note!("{}: {message}", self.label);
            *said = Some(message.clone());
        }
        self.last_error = Some(message);
    }

    /// Bring the open streams in line with what should be recorded at `now`.
    fn rescan(&mut self, now: Duration) {
        let procs = if self.spec.needs_processes() { self.table.snapshot() } else { Vec::new() };
        let plan = plan(&self.spec, &procs);
        if self.planned.as_ref() != Some(&plan) {
            for note in &plan.notes {
                crate::note!("{}: {note}", self.label);
            }
            if self.planned.is_some() {
                crate::note!("{}: now recording {:?}", self.label, plan.taps);
            }
        }
        let mut old = std::mem::take(&mut self.slots);
        for tap in &plan.taps {
            let default_id = self.default_of(tap);
            // A stream that is still the right one is kept as it is.
            let kept = old.iter().position(|s| s.tap == *tap && s.default_id == default_id).map(|at| old.remove(at));
            let mut slot = kept.unwrap_or_else(|| Slot { tap: tap.clone(), default_id, stream: None, said: None, failures: 0, retry_at: Duration::ZERO, buffer: TrackBuffer::default() });
            // One that would not open is left alone for longer each time;
            // one that was open and ended is opened again at this look.
            if slot.stream.is_none() && now >= slot.retry_at {
                slot.stream = self.open(tap, &mut slot.said);
                if slot.stream.is_some() {
                    slot.failures = 0;
                    if self.planned.is_some() {
                        self.recreations += 1;
                    }
                } else {
                    slot.failures += 1;
                    slot.retry_at = now + retry_delay(slot.failures);
                }
            }
            self.slots.push(slot);
        }
        for gone in old {
            add(&mut self.past, gone.buffer.stats);
        }
        self.planned = Some(plan);
    }
}

fn add(to: &mut TrackStats, more: TrackStats) {
    to.captured += more.captured;
    to.silence += more.silence;
    to.dropped += more.dropped;
    to.underruns += more.underruns;
}

impl Source for TrackSource {
    fn fill(&mut self, now: Duration, out: &mut [f32], owed: usize) {
        if now >= self.next_scan {
            self.next_scan = now + RESCAN;
            self.rescan(now);
        }
        out.fill(0.0);
        self.one.resize(out.len(), 0.0);
        let mut failed = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if let Some(stream) = slot.stream.as_mut() {
                self.scratch.clear();
                if let Err(err) = stream.read(&mut self.scratch) {
                    slot.stream = None;
                    failed.push((index, err));
                }
                slot.buffer.push(&self.scratch);
            }
            slot.buffer.take(&mut self.one);
            slot.buffer.trim(owed);
            for (sum, sample) in out.iter_mut().zip(&self.one) {
                *sum += sample;
            }
        }
        if self.slots.is_empty() {
            self.past.silence += (out.len() / crate::mixer::CHANNELS) as u64;
        }
        for (index, err) in failed {
            let tap = self.slots[index].tap.clone();
            let mut said = self.slots[index].said.take();
            self.say(&mut said, format!("{tap:?} stopped ({err}); it is opened again within {} s", RESCAN.as_secs()));
            self.slots[index].said = said;
        }
    }

    fn stats(&self) -> TrackStats {
        let mut all = self.past;
        for slot in &self.slots {
            add(&mut all, slot.buffer.stats);
        }
        all
    }

    fn status(&self) -> String {
        let open = self.slots.iter().filter(|s| s.stream.is_some()).count();
        let mut text = format!("{open} of {} streams open, {} re-creations", self.slots.len(), self.recreations);
        if let Some(err) = &self.last_error {
            text.push_str(&format!(", last problem: {err}"));
        }
        text
    }
}

/// How the samples of a stream are laid out, for turning a device's own
/// format into ours when Windows will not do it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub channels: usize,
    /// Bytes in one sample of one channel.
    pub bytes: usize,
    pub float: bool,
}

impl Layout {
    /// What Windows is asked for: stereo 32-bit floats.
    pub const OURS: Layout = Layout { channels: crate::mixer::CHANNELS, bytes: 4, float: true };

    /// Whether [`Layout::convert`] can read this.
    pub fn readable(&self) -> bool {
        self.channels >= 1 && if self.float { self.bytes == 4 } else { (2..=4).contains(&self.bytes) }
    }

    pub fn frame(&self) -> usize {
        self.channels * self.bytes
    }

    /// One sample as a float between -1 and 1.
    fn sample(&self, b: &[u8]) -> f32 {
        match (self.float, self.bytes) {
            (true, _) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (false, 2) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
            (false, 3) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2_147_483_648.0,
            (false, _) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
        }
    }

    /// Append `bytes` (whole frames of a readable layout) as stereo floats:
    /// the first two channels, or the one channel on both sides.
    pub fn convert(&self, bytes: &[u8], out: &mut Vec<f32>) {
        for frame in bytes.chunks_exact(self.frame()) {
            let left = self.sample(&frame[..self.bytes]);
            let right = if self.channels > 1 { self.sample(&frame[self.bytes..2 * self.bytes]) } else { left };
            out.extend([left, right]);
        }
    }
}

/// Gives nothing, so its track is silence: what every source is where
/// there is no Windows to ask for sound.
pub struct NoSound;

impl Opener for NoSound {
    fn open(&mut self, _tap: &Tap) -> Result<Box<dyn Stream>, String> {
        Ok(Box::new(NoSound))
    }
    fn default_id(&mut self, _output: bool) -> Option<String> {
        None
    }
}

impl Stream for NoSound {
    fn read(&mut self, _out: &mut Vec<f32>) -> Result<(), String> {
        Ok(())
    }
}

/// A steady tone, for trying the whole path without a sound device: each
/// track gets its own pitch, so the tracks of a clip can be told apart.
pub struct Tone {
    hz: f64,
    made: u64,
    started: std::time::Instant,
}

impl Tone {
    pub fn for_track(index: usize) -> Self {
        Self { hz: 440.0 * (index + 1) as f64, made: 0, started: std::time::Instant::now() }
    }
}

impl Opener for Tone {
    fn open(&mut self, _tap: &Tap) -> Result<Box<dyn Stream>, String> {
        Ok(Box::new(Tone { hz: self.hz, made: 0, started: std::time::Instant::now() }))
    }
    fn default_id(&mut self, _output: bool) -> Option<String> {
        None
    }
}

impl Stream for Tone {
    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), String> {
        let rate = crate::mixer::RATE as u64;
        let due = (self.started.elapsed().as_micros() as u64 * rate / 1_000_000).saturating_sub(self.made).min(rate);
        for n in self.made..self.made + due {
            let sample = (0.25 * (std::f64::consts::TAU * self.hz * n as f64 / rate as f64).sin()) as f32;
            out.extend([sample, sample]);
        }
        self.made += due;
        Ok(())
    }
}

/// The opener for this system and run: Windows' sound devices, or silence
/// (or test tones, when `ZENITH_REC_TEST_TONE` is set) without them.
pub fn opener(track: usize) -> Box<dyn Opener> {
    if std::env::var_os("ZENITH_REC_TEST_TONE").is_some() {
        return Box::new(Tone::for_track(track));
    }
    #[cfg(windows)]
    {
        Box::new(windows::Wasapi::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(NoSound)
    }
}

/// The sources of a recording's tracks, in order. Call this on the thread
/// that will pull from them.
pub fn sources(values: &[String], table: &Arc<ProcessTable>) -> Vec<Box<dyn Source>> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let spec = parse_spec(value);
            crate::note!("track {index}: {value} -> {spec:?}");
            Box::new(TrackSource::new(format!("track {index}"), spec, opener(index), table.clone())) as Box<dyn Source>
        })
        .collect()
}

/// The sound devices by the names `-a` takes, each with a name for people:
/// the two defaults, then the speakers (ending in `.monitor`), then the
/// microphones.
pub fn devices() -> Vec<(String, String)> {
    let mut list = vec![("default_output".to_string(), "Default output".to_string()), ("default_input".to_string(), "Default input".to_string())];
    #[cfg(windows)]
    list.extend(windows::devices());
    // A `|` or a line break in a label would break the line it is printed on.
    for (_, label) in &mut list {
        *label = label.replace(['|', '\n', '\r'], " ");
    }
    list.retain(|(name, _)| !name.contains(' ') && !name.contains('|'));
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    fn proc(pid: u32, parent: u32, name: &str) -> Proc {
        Proc { pid, parent: (parent != 0).then_some(parent), name: name.into() }
    }

    fn apps(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn the_daemons_sources_are_read() {
        assert_eq!(parse_spec("default_output"), Spec::DefaultOutput);
        assert_eq!(parse_spec("default_input"), Spec::DefaultInput);
        assert_eq!(parse_spec("{0.0.0.00000000}.{abc}.monitor"), Spec::Output("{0.0.0.00000000}.{abc}".into()));
        assert_eq!(parse_spec("{0.0.1.00000000}.{def}"), Spec::Input("{0.0.1.00000000}.{def}".into()));
        assert_eq!(parse_spec("app:Discord|app:Mumble"), Spec::Apps(apps(&["Discord", "Mumble"])));
        assert_eq!(parse_spec("app-inverse:Discord|app-inverse:Mumble"), Spec::AllExcept(apps(&["Discord", "Mumble"])));
        assert_eq!(parse_spec("app:Team Speak"), Spec::Apps(apps(&["Team Speak"])));
    }

    #[test]
    fn devices_joined_with_a_bar_are_added_together() {
        assert_eq!(parse_spec("default_output|default_input"), Spec::Several(vec![Spec::DefaultOutput, Spec::DefaultInput]));
        assert_eq!(parse_spec("app:Discord|default_input"), Spec::Several(vec![Spec::Apps(apps(&["Discord"])), Spec::DefaultInput]));
        assert_eq!(parse_spec(""), Spec::Several(vec![]));
        assert!(plan(&parse_spec(""), &[]).taps.is_empty());
    }

    #[test]
    fn the_defaults_and_named_devices_need_no_programs() {
        assert_eq!(plan(&Spec::DefaultOutput, &[]).taps, [Tap::Output(None)]);
        assert_eq!(plan(&Spec::DefaultInput, &[]).taps, [Tap::Input(None)]);
        assert_eq!(plan(&Spec::Output("x".into()), &[]).taps, [Tap::Output(Some("x".into()))]);
        assert_eq!(plan(&Spec::Input("m".into()), &[]).taps, [Tap::Input(Some("m".into()))]);
        assert!(!Spec::DefaultOutput.needs_processes());
        assert!(Spec::Apps(apps(&["a"])).needs_processes());
        assert!(Spec::Several(vec![Spec::DefaultInput, Spec::AllExcept(apps(&["a"]))]).needs_processes());
    }

    fn discord_running() -> Vec<Proc> {
        vec![
            proc(4, 0, "System"),
            proc(900, 4, "explorer.exe"),
            proc(1200, 900, "Discord.exe"),
            proc(1210, 1200, "Discord.exe"),
            proc(1220, 1200, "Discord.exe"),
            proc(2000, 900, "game.exe"),
        ]
    }

    #[test]
    fn the_chat_track_is_each_voice_programs_tree() {
        let spec = Spec::Apps(apps(&["Discord", "Mumble"]));
        assert_eq!(plan(&spec, &discord_running()).taps, [Tap::Tree { pid: 1200, include: true }]);
        let mut both = discord_running();
        both.push(proc(3000, 900, "mumble.exe"));
        assert_eq!(plan(&spec, &both).taps, [Tap::Tree { pid: 1200, include: true }, Tap::Tree { pid: 3000, include: true }]);
        // None of them running: a silent track, not an error.
        assert_eq!(plan(&spec, &[proc(2000, 900, "game.exe")]), Plan::default());
    }

    #[test]
    fn the_game_track_leaves_the_voice_program_out() {
        let spec = Spec::AllExcept(apps(&["Discord", "Mumble"]));
        let one = plan(&spec, &discord_running());
        assert_eq!(one.taps, [Tap::Tree { pid: 1200, include: false }]);
        assert!(one.notes.is_empty());
        // No voice program running: all system sound, the plain way.
        assert_eq!(plan(&spec, &[proc(2000, 900, "game.exe")]).taps, [Tap::Output(None)]);
    }

    #[test]
    fn two_voice_programs_at_once_are_said_in_the_log() {
        let spec = Spec::AllExcept(apps(&["Mumble", "Discord"]));
        let mut both = discord_running();
        both.push(proc(3000, 900, "mumble.exe"));
        let plan = plan(&spec, &both);
        // The first named in the settings is the one left out.
        assert_eq!(plan.taps, [Tap::Tree { pid: 3000, include: false }]);
        assert_eq!(plan.notes.len(), 1);
        assert!(plan.notes[0].contains("Discord.exe (1200)"), "{}", plan.notes[0]);
    }

    /// What a fake opener was asked and how it should answer.
    #[derive(Default)]
    struct Script {
        opened: Vec<Tap>,
        refuse: Vec<Tap>,
        default_output: Option<String>,
        /// What each opened stream gives on its next reads; an `Err` ends it.
        reads: VecDeque<Result<Vec<f32>, String>>,
    }

    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<Script>>);

    impl Opener for Fake {
        fn open(&mut self, tap: &Tap) -> Result<Box<dyn Stream>, String> {
            let mut script = self.0.borrow_mut();
            if script.refuse.contains(tap) {
                return Err("0x88890008".into());
            }
            script.opened.push(tap.clone());
            Ok(Box::new(self.clone()))
        }
        fn default_id(&mut self, output: bool) -> Option<String> {
            if output { self.0.borrow().default_output.clone() } else { None }
        }
    }

    impl Stream for Fake {
        fn read(&mut self, out: &mut Vec<f32>) -> Result<(), String> {
            match self.0.borrow_mut().reads.pop_front() {
                Some(Ok(samples)) => {
                    out.extend(samples);
                    Ok(())
                }
                Some(Err(err)) => Err(err),
                None => Ok(()),
            }
        }
    }

    fn source(spec: Spec, procs: Vec<Proc>) -> (TrackSource, Fake, Arc<ProcessTable>) {
        let fake = Fake::default();
        let table = Arc::new(ProcessTable::default());
        table.replace(procs);
        (TrackSource::new("track 0".into(), spec, Box::new(fake.clone()), table.clone()), fake, table)
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// Enough of `value` for a stream to start playing 480 frames at once.
    fn start_of(value: f32) -> Vec<f32> {
        vec![value; (480 + crate::mixer::CUSHION) * 2]
    }

    /// A wake-up with nothing due: the streams are looked after all the same.
    fn look(track: &mut TrackSource, now: Duration) {
        track.fill(now, &mut [], 0);
    }

    #[test]
    fn a_voice_program_starting_and_ending_changes_what_is_open() {
        let (mut track, fake, table) = source(Spec::AllExcept(apps(&["Discord"])), vec![proc(2000, 900, "game.exe")]);
        look(&mut track, secs(0));
        assert_eq!(fake.0.borrow().opened, [Tap::Output(None)]);
        // Discord starts; nothing changes until the next look.
        table.replace(discord_running());
        look(&mut track, secs(1));
        assert_eq!(fake.0.borrow().opened.len(), 1);
        look(&mut track, secs(3));
        assert_eq!(fake.0.borrow().opened[1], Tap::Tree { pid: 1200, include: false });
        // It ends again.
        table.replace(vec![proc(2000, 900, "game.exe")]);
        look(&mut track, secs(6));
        assert_eq!(fake.0.borrow().opened[2], Tap::Output(None));
        assert_eq!(track.recreations, 2);
        // Nothing new: nothing opened.
        look(&mut track, secs(9));
        assert_eq!(fake.0.borrow().opened.len(), 3);
    }

    #[test]
    fn leaving_a_program_out_falls_back_to_everything() {
        let (mut track, fake, _table) = source(Spec::AllExcept(apps(&["Discord"])), discord_running());
        fake.0.borrow_mut().refuse.push(Tap::Tree { pid: 1200, include: false });
        fake.0.borrow_mut().reads.push_back(Ok(start_of(0.5)));
        let mut out = vec![9.0; 960];
        track.fill(secs(0), &mut out, 0);
        assert_eq!(fake.0.borrow().opened, [Tap::Output(None)]);
        assert!(out.iter().all(|&s| s == 0.5), "the fallback's sound is the track's");
        assert!(track.status().contains("could not leave program 1200 out"), "{}", track.status());
    }

    #[test]
    fn a_source_that_will_not_open_is_silent_and_tried_again() {
        let (mut track, fake, _table) = source(Spec::DefaultInput, vec![]);
        fake.0.borrow_mut().refuse.push(Tap::Input(None));
        look(&mut track, secs(0));
        look(&mut track, secs(1));
        let mut out = vec![9.0; 960];
        track.fill(secs(1), &mut out, 0);
        assert!(out.iter().all(|&s| s == 0.0));
        assert!(track.status().starts_with("0 of 1 streams open"), "{}", track.status());
        // The microphone is plugged in.
        fake.0.borrow_mut().refuse.clear();
        look(&mut track, secs(3));
        assert_eq!(fake.0.borrow().opened, [Tap::Input(None)]);
        assert!(track.status().starts_with("1 of 1 streams open, 1 re-creations"), "{}", track.status());
    }

    #[test]
    fn a_source_that_keeps_failing_is_tried_less_and_less_often() {
        assert_eq!(retry_delay(1), RESCAN);
        assert_eq!(retry_delay(2), RESCAN * 2);
        assert_eq!(retry_delay(4), RESCAN * 8);
        assert_eq!(retry_delay(6), MAX_RETRY);
        assert_eq!(retry_delay(u32::MAX), MAX_RETRY);
        // Counted by asking the opener for a tap it refuses: every look is
        // three seconds after the last, for ten minutes.
        struct Counting(Rc<RefCell<Vec<u64>>>, Rc<RefCell<bool>>);
        impl Opener for Counting {
            fn open(&mut self, _tap: &Tap) -> Result<Box<dyn Stream>, String> {
                if *self.1.borrow() {
                    return Ok(Box::new(NoSound));
                }
                Err("0x88890004".into())
            }
            fn default_id(&mut self, _output: bool) -> Option<String> {
                // Asked once at every look, before the stream is opened or left.
                let mut looks = self.0.borrow_mut();
                let look = looks.len() as u64;
                looks.push(look);
                None
            }
        }
        let (looks, plugged_in) = (Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(false)));
        let table = Arc::new(ProcessTable::default());
        let mut track = TrackSource::new("track 0".into(), Spec::DefaultInput, Box::new(Counting(looks.clone(), plugged_in.clone())), table);
        let mut tried_at = Vec::new();
        for look_at in (0..600).step_by(3) {
            look(&mut track, secs(look_at));
            if track.slots[0].retry_at > secs(look_at) && tried_at.last() != Some(&track.slots[0].retry_at) {
                tried_at.push(track.slots[0].retry_at);
            }
        }
        assert_eq!(looks.borrow().len(), 200, "it is looked at as often as ever");
        let waits: Vec<u64> = std::iter::once(0).chain(tried_at.iter().map(Duration::as_secs)).collect::<Vec<_>>().windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(waits[..6], [3, 6, 12, 24, 48, 60], "{waits:?}");
        assert!(waits[6..].iter().all(|&wait| wait == 60), "{waits:?}");
        assert!(tried_at.len() < 20, "a try every three seconds would have been 200: {}", tried_at.len());
        // Plugged in: open at the next try, and the count starts over.
        *plugged_in.borrow_mut() = true;
        let next = *tried_at.last().unwrap();
        look(&mut track, next);
        assert!(track.slots[0].stream.is_some());
        assert_eq!(track.slots[0].failures, 0);
    }

    #[test]
    fn a_stream_that_ends_is_opened_again_at_the_next_look() {
        let (mut track, fake, _table) = source(Spec::DefaultOutput, vec![]);
        fake.0.borrow_mut().reads.extend([Ok(start_of(0.1)), Err("device unplugged".into())]);
        let mut out = vec![9.0; 960];
        track.fill(secs(0), &mut out, 0);
        assert!(out.iter().all(|&s| s == 0.1));
        // What it gave before it ended is still played.
        track.fill(secs(1), &mut out, 0);
        assert!(out.iter().all(|&s| s == 0.1));
        assert!(track.status().contains("device unplugged"), "{}", track.status());
        look(&mut track, secs(2));
        assert_eq!(fake.0.borrow().opened.len(), 1, "not before the next look");
        look(&mut track, secs(3));
        assert_eq!(fake.0.borrow().opened.len(), 2);
    }

    #[test]
    fn new_default_speakers_are_followed() {
        let (mut track, fake, _table) = source(Spec::DefaultOutput, vec![]);
        fake.0.borrow_mut().default_output = Some("speakers".into());
        look(&mut track, secs(0));
        look(&mut track, secs(3));
        assert_eq!(fake.0.borrow().opened.len(), 1, "the same default: nothing to do");
        fake.0.borrow_mut().default_output = Some("headset".into());
        look(&mut track, secs(6));
        assert_eq!(fake.0.borrow().opened.len(), 2);
    }

    #[test]
    fn the_stand_in_for_leaving_a_program_out_follows_the_default_speakers_too() {
        let (mut track, fake, _table) = source(Spec::AllExcept(apps(&["Discord"])), discord_running());
        fake.0.borrow_mut().refuse.push(Tap::Tree { pid: 1200, include: false });
        fake.0.borrow_mut().default_output = Some("speakers".into());
        look(&mut track, secs(0));
        look(&mut track, secs(3));
        assert_eq!(fake.0.borrow().opened, [Tap::Output(None)], "the same speakers: nothing to do");
        fake.0.borrow_mut().default_output = Some("headset".into());
        look(&mut track, secs(6));
        assert_eq!(fake.0.borrow().opened, [Tap::Output(None), Tap::Output(None)]);
    }

    #[test]
    fn two_voice_programs_are_added_together() {
        let mut procs = discord_running();
        procs.push(proc(3000, 900, "mumble.exe"));
        let (mut track, fake, _table) = source(Spec::Apps(apps(&["Discord", "Mumble"])), procs);
        // Each gives its sound in its own time: the first all at once,
        // the second a wake-up later. Each is heard from its own start.
        fake.0.borrow_mut().reads.extend([Ok(start_of(0.25)), Ok(Vec::new()), Ok(Vec::new()), Ok(start_of(0.5))]);
        let mut out = vec![9.0; 960];
        track.fill(secs(0), &mut out, 0);
        assert!(out.iter().all(|&s| s == 0.25), "only the first has begun");
        track.fill(secs(1), &mut out, 0);
        assert!(out.iter().all(|&s| s == 0.75), "added sample by sample");
        let stats = track.stats();
        assert_eq!((stats.captured, stats.silence), (2 * (480 + crate::mixer::CUSHION) as u64, 480));
    }

    #[test]
    fn a_track_with_no_stream_is_silence_that_is_counted() {
        let (mut track, _fake, table) = source(Spec::Apps(apps(&["Discord"])), vec![proc(2000, 900, "game.exe")]);
        let mut out = vec![9.0; 960];
        track.fill(secs(0), &mut out, 0);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(track.stats().silence, 480);
        // The counts of a stream that is gone are kept.
        table.replace(discord_running());
        track.fill(secs(3), &mut out, 0);
        table.replace(vec![proc(2000, 900, "game.exe")]);
        track.fill(secs(6), &mut out, 0);
        assert_eq!(track.stats().silence, 3 * 480);
    }

    #[test]
    fn a_devices_own_format_is_turned_into_stereo_floats() {
        let mut out = Vec::new();
        // Ours already: copied.
        let floats: Vec<u8> = [0.5f32, -0.25].iter().flat_map(|s| s.to_le_bytes()).collect();
        Layout::OURS.convert(&floats, &mut out);
        assert_eq!(out, [0.5, -0.25]);
        // 16-bit 5.1: the front pair is kept.
        out.clear();
        let six: Vec<u8> = [16384i16, -32768, 1, 2, 3, 4].iter().flat_map(|s| s.to_le_bytes()).collect();
        Layout { channels: 6, bytes: 2, float: false }.convert(&six, &mut out);
        assert_eq!(out, [0.5, -1.0]);
        // 24-bit mono: on both sides, with its sign.
        out.clear();
        Layout { channels: 1, bytes: 3, float: false }.convert(&[0x00, 0x00, 0xC0, 0x00, 0x00, 0x40], &mut out);
        assert_eq!(out, [-0.5, -0.5, 0.5, 0.5]);
        // 32-bit integers.
        out.clear();
        let ints: Vec<u8> = [i32::MIN, 1 << 30].iter().flat_map(|s| s.to_le_bytes()).collect();
        Layout { channels: 2, bytes: 4, float: false }.convert(&ints, &mut out);
        assert_eq!(out, [-1.0, 0.5]);
        assert!(Layout::OURS.readable());
        assert!(!Layout { channels: 2, bytes: 8, float: true }.readable());
        assert!(!Layout { channels: 0, bytes: 2, float: false }.readable());
        assert!(!Layout { channels: 2, bytes: 1, float: false }.readable());
    }

    #[test]
    fn the_listed_devices_start_with_the_defaults() {
        let list = devices();
        assert_eq!(list[0], ("default_output".to_string(), "Default output".to_string()));
        assert_eq!(list[1], ("default_input".to_string(), "Default input".to_string()));
    }

    #[test]
    fn a_tone_keeps_time() {
        let mut tone = Tone::for_track(0);
        std::thread::sleep(Duration::from_millis(50));
        let mut out = Vec::new();
        tone.read(&mut out).unwrap();
        let frames = out.len() / 2;
        assert!((2400..12_000).contains(&frames), "{frames} frames in about 50 ms");
        assert!(out.iter().any(|s| s.abs() > 0.1));
    }
}
