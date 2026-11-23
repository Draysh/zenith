//! The sound ffmpeg is fed: every track side by side in one stream of
//! 48 kHz stereo floats, written at exactly the speed of the clock.
//!
//! ffmpeg lays the stream against the picture by counting samples, so the
//! count is what keeps sound and picture together. A source that has
//! nothing to give (nothing is playing, the device went away) is filled
//! with silence; one that gives more than the clock allows loses the
//! excess. Neither may ever hold the others up.
//!
//! Nor may ffmpeg: it takes the stream in bursts, as its picture comes,
//! and a mixer that waited for it would not be reading the sound devices
//! meanwhile, whose buffers then overflow. So the stream is handed to a
//! thread of its own that does the waiting.

use crate::audio::Source;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// Frames a second, of every track.
pub const RATE: u32 = 48_000;
/// Samples in a frame of one track.
pub const CHANNELS: usize = 2;

/// How often the mixer wakes.
const TICK: Duration = Duration::from_millis(10);
/// The most made in one go, so that catching up after a pause comes in
/// pieces.
const MAX_CHUNK: usize = RATE as usize / 10;
/// Falling this far behind the clock (the machine slept, ffmpeg stopped
/// reading) is not caught up with sound: the time is skipped instead, and
/// the [`Sink`] puts silence in its place, so that what comes after is
/// still where it belongs.
const LATE_LIMIT: u64 = RATE as u64 * 2;
/// How much a track holds beyond what is taken when it starts to play:
/// two of the 10 ms packets sound devices deliver. The clock takes sound
/// evenly and a device gives it a packet at a time, so what is held swings
/// by a packet; with two to spare, the swing never reaches the bottom and
/// a packet arriving late is not a gap.
pub const CUSHION: usize = RATE as usize / 50;
/// A track holding more than this is running ahead of the clock.
pub const MAX_QUEUED: usize = RATE as usize / 5;
/// Wake-ups after which a few samples that never became enough to play are stale.
const STALE_TAKES: u32 = 20;

/// Says how many frames are due, from the time since the start.
#[derive(Debug, Default)]
pub struct Pacer {
    /// Frames accounted for: written or skipped.
    done: u64,
    skipped: u64,
}

impl Pacer {
    fn target(elapsed: Duration) -> u64 {
        (elapsed.as_nanos() * RATE as u128 / 1_000_000_000) as u64
    }

    /// The frames to write now. They count as written.
    pub fn due(&mut self, elapsed: Duration) -> usize {
        let mut behind = Self::target(elapsed).saturating_sub(self.done);
        if behind > LATE_LIMIT {
            let skip = behind - TICK.as_millis() as u64 * RATE as u64 / 1000;
            self.skipped += skip;
            self.done += skip;
            behind -= skip;
        }
        let frames = behind.min(MAX_CHUNK as u64);
        self.done += frames;
        frames as usize
    }

    /// The frames still due after the last [`Pacer::due`]: what a pause
    /// left to catch up on.
    pub fn owed(&self, elapsed: Duration) -> usize {
        Self::target(elapsed).saturating_sub(self.done) as usize
    }

    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Frames written, which is frames accounted for less those skipped.
    pub fn written(&self) -> u64 {
        self.done - self.skipped
    }
}

/// What became of one track's sound, in frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackStats {
    /// Received from the source.
    pub captured: u64,
    /// Written as silence because the source had nothing.
    pub silence: u64,
    /// Thrown away because the source ran ahead of the clock.
    pub dropped: u64,
    /// Times the track ran dry while it was playing.
    pub underruns: u64,
}

/// One track's sound between its source and the stream.
#[derive(Debug, Default)]
pub struct TrackBuffer {
    queue: VecDeque<f32>,
    /// Playing: enough was queued to start, and it has not run dry since.
    playing: bool,
    /// Wake-ups in a row with something queued but not enough to play.
    waiting: u32,
    pub stats: TrackStats,
}

impl TrackBuffer {
    fn queued(&self) -> usize {
        self.queue.len() / CHANNELS
    }

    /// Take in what the source captured: stereo, interleaved.
    pub fn push(&mut self, samples: &[f32]) {
        self.queue.extend(samples);
        self.stats.captured += (samples.len() / CHANNELS) as u64;
    }

    /// Throw away what the source gave beyond what the clock will take:
    /// `owed` frames are still due after a pause, and what is held for
    /// those is not excess.
    pub fn trim(&mut self, owed: usize) {
        if self.queued() > MAX_QUEUED + owed {
            // Down to the cushion in one go: one audible skip now and then,
            // not a little lost at every wake-up.
            let excess = self.queued() - (CUSHION + owed);
            self.queue.drain(..excess * CHANNELS);
            self.stats.dropped += excess as u64;
        }
    }

    /// Fill `out` (stereo, interleaved) with the next frames of the track,
    /// silence where there are none.
    pub fn take(&mut self, out: &mut [f32]) {
        let frames = out.len() / CHANNELS;
        if !self.playing {
            if self.queued() >= frames + CUSHION {
                self.playing = true;
            } else if !self.queue.is_empty() {
                self.waiting += 1;
                if self.waiting > STALE_TAKES {
                    // The tail of a sound that ended: played later it would
                    // be in the wrong place.
                    self.stats.dropped += self.queued() as u64;
                    self.queue.clear();
                }
            }
        }
        let have = if self.playing { self.queued().min(frames) } else { 0 };
        for (slot, sample) in out.iter_mut().zip(self.queue.drain(..have * CHANNELS)) {
            *slot = sample;
        }
        out[have * CHANNELS..].fill(0.0);
        self.stats.silence += (frames - have) as u64;
        if self.playing {
            self.waiting = 0;
            if have < frames {
                self.playing = false;
                self.stats.underruns += 1;
            }
        }
    }
}

/// Every track's source, and the clock they are written by.
pub struct Mixer {
    sources: Vec<Box<dyn Source>>,
    pacer: Pacer,
    one: Vec<f32>,
    block: Vec<u8>,
    /// The skipped frames [`Mixer::take_skipped`] has told of already.
    told: u64,
}

impl Mixer {
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self {
        Self { sources, pacer: Pacer::default(), one: Vec::new(), block: Vec::new(), told: 0 }
    }

    /// The bytes to write at `elapsed` since the start: the frames now due,
    /// each a stereo pair for every track in order, as little-endian floats.
    /// Empty when nothing is due yet.
    pub fn tick(&mut self, elapsed: Duration) -> &[u8] {
        let frames = self.pacer.due(elapsed);
        let owed = self.pacer.owed(elapsed);
        let width = self.sources.len() * CHANNELS;
        self.block.clear();
        self.block.resize(frames * width * 4, 0);
        self.one.resize(frames * CHANNELS, 0.0);
        for (index, source) in self.sources.iter_mut().enumerate() {
            // Asked even when nothing is due: the devices are read either way.
            source.fill(elapsed, &mut self.one, owed);
            for (frame, pair) in self.one.as_chunks::<CHANNELS>().0.iter().enumerate() {
                for (channel, sample) in pair.iter().enumerate() {
                    let at = (frame * width + index * CHANNELS + channel) * 4;
                    self.block[at..at + 4].copy_from_slice(&sample.to_le_bytes());
                }
            }
        }
        &self.block
    }

    /// The frames the clock skipped since this was last asked: time that
    /// passed with nothing written for it.
    pub fn take_skipped(&mut self) -> u64 {
        let new = self.pacer.skipped() - self.told;
        self.told += new;
        new
    }

    /// Bytes in one frame of the stream: a stereo pair for every track.
    pub fn frame_bytes(&self) -> usize {
        self.sources.len() * CHANNELS * 4
    }

    /// Bytes in a second of the stream.
    pub fn bytes_a_second(&self) -> usize {
        self.frame_bytes() * RATE as usize
    }

    /// A line for the log about each track since the start.
    pub fn report(&self, elapsed: Duration) -> Vec<String> {
        let mut lines =
            vec![format!("sound: {} frames made in {:.1} s ({} skipped)", self.pacer.written(), elapsed.as_secs_f64(), self.pacer.skipped())];
        for (index, source) in self.sources.iter().enumerate() {
            let s = source.stats();
            lines.push(format!(
                "track {index}: captured {} silence {} dropped {} underruns {}; {}",
                s.captured,
                s.silence,
                s.dropped,
                s.underruns,
                source.status()
            ));
        }
        lines
    }

    pub fn stats(&self) -> Vec<TrackStats> {
        self.sources.iter().map(|source| source.stats()).collect()
    }
}

/// A pipe takes this much before anyone reads it (Linux, and the pipes
/// Rust makes on Windows): more than this written means ffmpeg is reading.
const PIPE_ROOM: usize = 64 * 1024;
/// A write that took this long was waiting for ffmpeg to read.
const WAITED: Duration = Duration::from_millis(20);
/// How much sound may wait for ffmpeg to take it, in seconds of stream.
/// It reads in bursts, a tenth of a second apart at the worst; beyond
/// this it has stopped, and what it has not taken by then is left out,
/// with as much silence owed in its place.
const MAX_WAITING_SECS: usize = 2;
/// The most silence handed over in one go when making up for sound that
/// was left out, as a share of what may wait: half a second, so a long
/// debt is paid in pieces and never held in memory whole.
const SILENCE_SHARE: usize = 4;
/// Owing this many times what may wait (half a minute of sound) is past
/// making up for: the recorder starts that ffmpeg afresh instead.
const MAX_OWED: usize = 15;

/// What the mixer and the thread writing to one ffmpeg both know.
#[derive(Default)]
struct Flow {
    /// Bytes handed over and not yet written.
    waiting: AtomicUsize,
    /// ffmpeg has begun to read.
    reading: AtomicBool,
    /// Bytes the pipe took before ffmpeg read any: sound from before its
    /// picture began, which it reads first all the same. As much of what
    /// comes next is left out, so that everything after is in its place.
    stale: AtomicUsize,
    /// The pipe is closed: ffmpeg is gone.
    broken: AtomicBool,
}

struct Writer {
    blocks: mpsc::Sender<Vec<u8>>,
    flow: Arc<Flow>,
    /// Bytes of sound this ffmpeg was due and never given, since it began
    /// to read: it counts samples, so each one missing would put all that
    /// follows too early. As many are written as silence as soon as it
    /// takes them, and only then is sound passed on again.
    owed: usize,
    /// More is owed than is worth making up for.
    fell_behind: bool,
}

/// Write what arrives on `blocks` to ffmpeg, waiting as long as each
/// write takes; closing the pipe when there is no more to come, which is
/// also how ffmpeg learns the sound has ended.
fn write_blocks(mut pipe: impl Write, blocks: mpsc::Receiver<Vec<u8>>, flow: Arc<Flow>) {
    let mut written = 0usize;
    for block in blocks {
        let began = Instant::now();
        let result = pipe.write_all(&block);
        flow.waiting.fetch_sub(block.len(), Ordering::Relaxed);
        if let Err(err) = result {
            crate::note!("sound: ffmpeg stopped taking it ({err})");
            flow.broken.store(true, Ordering::Relaxed);
            return;
        }
        written += block.len();
        if flow.reading.load(Ordering::Relaxed) {
            continue;
        }
        if began.elapsed() >= WAITED {
            // The pipe was full and this write waited for ffmpeg: all of
            // it so far was written before ffmpeg began.
            flow.stale.store(written, Ordering::Relaxed);
            flow.reading.store(true, Ordering::Relaxed);
        } else if written > PIPE_ROOM {
            // It never had to wait: ffmpeg was reading from the start.
            flow.reading.store(true, Ordering::Relaxed);
        }
    }
}

/// What became of a block given to the [`Sink`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// There is no ffmpeg to give it to.
    Nowhere,
    /// On its way.
    Passed,
    /// Left out: ffmpeg has not begun to read, and its sound starts when
    /// it does, not when we began to wait for it.
    NotReadingYet,
    /// Left out: ffmpeg has stopped taking the sound. Silence is owed for it.
    Stuck,
    /// Left out, with silence on its way in its place: what was owed is
    /// being made up for, and sound is passed on again once it is.
    CatchingUp,
}

/// Where the mixer writes: the current ffmpeg's stdin, through a thread
/// that waits for ffmpeg so the mixer never does. The recorder puts a new
/// pipe here each time it starts ffmpeg.
pub struct Sink {
    writer: Option<Writer>,
    /// Bytes in one frame of the stream: silence is made of whole frames.
    frame: usize,
}

impl Default for Sink {
    fn default() -> Self {
        Self::for_tracks(1)
    }
}

impl Sink {
    /// The sink of a stream of `tracks` tracks.
    pub fn for_tracks(tracks: usize) -> Self {
        Self { writer: None, frame: tracks.max(1) * CHANNELS * 4 }
    }

    /// Write to `pipe` from now on: ffmpeg's stdin. What was owed to the
    /// ffmpeg before is forgotten, as this one counts from its own start.
    pub fn connect(&mut self, pipe: impl Write + Send + 'static) {
        let (blocks, incoming) = mpsc::channel();
        let flow = Arc::new(Flow::default());
        let theirs = flow.clone();
        std::thread::spawn(move || write_blocks(pipe, incoming, theirs));
        self.writer = Some(Writer { blocks, flow, owed: 0, fell_behind: false });
    }

    /// Let go of the pipe: its thread closes it once it is not in a write.
    pub fn disconnect(&mut self) {
        self.writer = None;
    }

    /// Whether the ffmpeg connected now has begun to read.
    pub fn reading(&self) -> bool {
        self.writer.as_ref().is_some_and(|writer| writer.flow.reading.load(Ordering::Relaxed))
    }

    /// Bytes of silence owed to the ffmpeg connected now.
    pub fn owed(&self) -> usize {
        self.writer.as_ref().map_or(0, |writer| writer.owed)
    }

    /// Whether the ffmpeg connected now is owed more sound than is worth
    /// making up for: it is to be started afresh.
    pub fn fell_behind(&self) -> bool {
        self.writer.as_ref().is_some_and(|writer| writer.fell_behind)
    }

    /// `bytes` of sound were never made, for time that passed all the same
    /// (the clock skipped it): owed as silence to an ffmpeg that is
    /// reading, whose count of samples is its clock. One that has not
    /// begun to read is owed nothing: its sound starts when it does.
    pub fn owe(&mut self, bytes: usize) {
        if let Some(writer) = &mut self.writer
            && writer.flow.reading.load(Ordering::Relaxed)
        {
            writer.owed += bytes;
        }
    }

    /// Hand `block` over, unless it is to be left out; never waits.
    /// `limit` is how many bytes may be waiting for ffmpeg. What is left
    /// out of an ffmpeg that is reading is owed to it, and paid in silence
    /// before any more sound: every byte it is due it gets, as one or the
    /// other, so its count of samples keeps to the clock.
    pub fn send(&mut self, block: &[u8], limit: usize) -> Sent {
        let sent = self.writer.as_mut().map_or(Sent::Nowhere, |writer| writer.send(block, limit, self.frame));
        if sent == Sent::Nowhere {
            self.writer = None;
        }
        sent
    }
}

impl Writer {
    /// Put `bytes` on the way to ffmpeg. `false` when its thread is gone.
    fn pass(&self, bytes: Vec<u8>) -> bool {
        self.flow.waiting.fetch_add(bytes.len(), Ordering::Relaxed);
        self.blocks.send(bytes).is_ok()
    }

    fn send(&mut self, block: &[u8], limit: usize, frame: usize) -> Sent {
        if self.flow.broken.load(Ordering::Relaxed) {
            return Sent::Nowhere;
        }
        if block.is_empty() {
            return Sent::Passed;
        }
        let waiting = self.flow.waiting.load(Ordering::Relaxed);
        if !self.flow.reading.load(Ordering::Relaxed) {
            // One block at a time until it reads, so that what it finds
            // when it begins is from that moment.
            if waiting > 0 {
                return Sent::NotReadingYet;
            }
            return if self.pass(block.to_vec()) { Sent::Passed } else { Sent::Nowhere };
        }
        if waiting + block.len() > limit {
            self.owe(block.len(), limit);
            return Sent::Stuck;
        }
        let stale = self.flow.stale.load(Ordering::Relaxed);
        if stale > 0 {
            self.flow.stale.store(stale.saturating_sub(block.len()), Ordering::Relaxed);
            return Sent::NotReadingYet;
        }
        // Less than a frame is not owed: only blocks of whole frames are.
        if self.owed < frame {
            self.owed = 0;
            return if self.pass(block.to_vec()) { Sent::Passed } else { Sent::Nowhere };
        }
        // Silence first, as much as there is room for and no more than a
        // share at a time: it stands where the sound left out would have.
        let pay = self.owed.min(limit - waiting).min((limit / SILENCE_SHARE).max(frame));
        let pay = pay - pay % frame;
        if pay > 0 {
            if !self.pass(vec![0; pay]) {
                return Sent::Nowhere;
            }
            self.owed -= pay;
        }
        if self.owed < frame && waiting + pay + block.len() <= limit {
            self.owed = 0;
            return if self.pass(block.to_vec()) { Sent::Passed } else { Sent::Nowhere };
        }
        // Still owing: this block's moment comes before the silence is all
        // written, so it would be heard too early. It is owed as well.
        self.owe(block.len(), limit);
        Sent::CatchingUp
    }

    fn owe(&mut self, bytes: usize, limit: usize) {
        self.owed += bytes;
        if self.owed > limit.saturating_mul(MAX_OWED) {
            self.fell_behind = true;
        }
    }
}

/// How often the counters go into the log.
const REPORT_EVERY: Duration = Duration::from_secs(10);

/// Mix until `stop` is set. `make_sources` is called here, on the mixer's
/// own thread, because the Windows sound interfaces must be used on the
/// thread that opened them.
/// One wake-up of the mixer at `elapsed` since its start: make what is
/// due and hand it to the sink, with what the clock skipped owed to it as
/// silence first.
fn hand_over(mixer: &mut Mixer, sink: &Mutex<Sink>, elapsed: Duration, limit: usize) -> Sent {
    mixer.tick(elapsed);
    let skipped = mixer.take_skipped() as usize * mixer.frame_bytes();
    if skipped > 0 {
        crate::note!("sound: the clock ran {} ms ahead of the mixer; silence stands in for that time", skipped / (mixer.bytes_a_second() / 1000).max(1));
    }
    sink.lock().map_or(Sent::Nowhere, |mut sink| {
        sink.owe(skipped);
        sink.send(&mixer.block, limit)
    })
}

pub fn run(make_sources: impl FnOnce() -> Vec<Box<dyn Source>>, sink: Arc<Mutex<Sink>>, stop: Arc<AtomicBool>) {
    // A game takes every processor it can; the sound must still be read on time.
    crate::sys::prefer_this_thread();
    let mut mixer = Mixer::new(make_sources());
    let limit = mixer.bytes_a_second() * MAX_WAITING_SECS;
    let start = Instant::now();
    let mut reported = Duration::ZERO;
    // Since when blocks have been left out, and for which reason.
    let mut left_out: Option<(Instant, Sent)> = None;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(TICK);
        let sent = hand_over(&mut mixer, &sink, start.elapsed(), limit);
        match (sent, left_out) {
            (Sent::NotReadingYet | Sent::Stuck | Sent::CatchingUp, None) => {
                if sent == Sent::Stuck {
                    crate::note!("sound: ffmpeg has not taken any for {MAX_WAITING_SECS} s; it is left out until it does, and silence put in its place then");
                }
                left_out = Some((Instant::now(), sent));
            }
            (Sent::Passed, Some((since, why))) => {
                if why == Sent::NotReadingYet {
                    crate::note!("sound: ffmpeg began reading; {} ms of waiting left out", since.elapsed().as_millis());
                } else {
                    crate::note!("sound: ffmpeg has all it was owed again; {} ms of sound were left out and are silence", since.elapsed().as_millis());
                }
                left_out = None;
            }
            (Sent::Nowhere, Some(_)) => left_out = None,
            _ => {}
        }
        if start.elapsed() - reported >= REPORT_EVERY {
            reported = start.elapsed();
            for line in mixer.report(reported) {
                crate::note!("{line}");
            }
        }
    }
    for line in mixer.report(start.elapsed()) {
        crate::note!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::ChildStdin;
    use std::sync::mpsc;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_second_is_exactly_48000_frames_however_it_is_cut_up() {
        for step_us in [10_000u64, 15_625, 3_333, 9_999, 31_000] {
            let mut pacer = Pacer::default();
            let mut total = 0;
            let mut now = 0;
            while now < 1_000_000 {
                now = (now + step_us).min(1_000_000);
                total += pacer.due(Duration::from_micros(now));
            }
            assert_eq!(total, 48_000, "step {step_us}");
            assert_eq!(pacer.skipped(), 0);
        }
    }

    #[test]
    fn a_short_pause_is_caught_up_in_pieces() {
        let mut pacer = Pacer::default();
        assert_eq!(pacer.due(ms(10)), 480);
        // Half a second without a wake-up: owed in full, a chunk at a time.
        assert_eq!(pacer.due(ms(510)), MAX_CHUNK);
        let mut total = 480 + MAX_CHUNK;
        while total < 24_480 {
            total += pacer.due(ms(510));
        }
        assert_eq!(total, 24_480);
        assert_eq!(pacer.due(ms(510)), 0);
        assert_eq!(pacer.skipped(), 0);
    }

    #[test]
    fn a_long_pause_is_skipped_not_caught_up() {
        let mut pacer = Pacer::default();
        pacer.due(ms(10));
        // Five seconds gone (the machine slept): only a wake-up's worth is written.
        assert_eq!(pacer.due(ms(5010)), 480);
        assert_eq!(pacer.skipped(), 5 * 48_000 - 480);
        assert_eq!(pacer.due(ms(5020)), 480);
        assert_eq!(pacer.written(), 3 * 480);
    }

    #[test]
    fn what_a_pause_left_to_catch_up_on_is_known() {
        let mut pacer = Pacer::default();
        assert_eq!(pacer.due(ms(10)), 480);
        assert_eq!(pacer.owed(ms(10)), 0);
        assert_eq!(pacer.due(ms(310)), MAX_CHUNK);
        assert_eq!(pacer.owed(ms(310)), 300 * 48 - MAX_CHUNK);
    }

    fn frames(n: usize, value: f32) -> Vec<f32> {
        vec![value; n * CHANNELS]
    }

    #[test]
    fn a_track_with_nothing_is_silence() {
        let mut track = TrackBuffer::default();
        let mut out = frames(480, 9.0);
        track.take(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
        assert_eq!(track.stats, TrackStats { silence: 480, ..Default::default() });
    }

    #[test]
    fn a_track_starts_with_two_packets_to_spare_and_then_plays_without_gaps() {
        let mut track = TrackBuffer::default();
        let mut out = frames(480, 9.0);
        for _ in 0..2 {
            track.push(&frames(480, 0.5));
            track.take(&mut out);
            assert!(out.iter().all(|&s| s == 0.0), "what is taken and two packets more is what it starts on");
        }
        track.push(&frames(480, 0.5));
        // From here a packet a wake-up keeps it fed, even two arriving late.
        for round in 0..100 {
            track.take(&mut out);
            assert!(out.iter().all(|&s| s == 0.5));
            if round != 50 && round != 51 {
                track.push(&frames(480, 0.5));
            }
            if round == 52 {
                track.push(&frames(960, 0.5));
            }
        }
        assert_eq!(track.stats.underruns, 0);
        assert_eq!(track.stats.silence, 960);
        assert_eq!(track.stats.dropped, 0);
    }

    #[test]
    fn a_packet_at_a_time_against_an_even_clock_never_runs_dry() {
        // A device gives 10 ms at a time; the clock takes whatever time has
        // passed, at wake-ups a little further apart. Wherever the two
        // start against each other, the swing stays above the bottom.
        for phase_us in (0..10_000).step_by(250) {
            let mut track = TrackBuffer::default();
            let (mut given, mut taken) = (0u64, 0u64);
            let mut now_us = 0u64;
            while now_us < 5_000_000 {
                now_us += 10_500;
                let packets = (now_us + phase_us) / 10_000;
                while given < packets {
                    track.push(&frames(480, 0.5));
                    given += 1;
                }
                let due = now_us * 48 / 1000 - taken;
                taken += due;
                track.take(&mut frames(due as usize, 0.0));
            }
            assert_eq!(track.stats.underruns, 0, "phase {phase_us}");
        }
    }

    #[test]
    fn running_dry_plays_what_is_left_then_silence_and_starts_over() {
        let mut track = TrackBuffer::default();
        track.push(&frames(1480, 0.5));
        let mut out = frames(480, 9.0);
        for _ in 0..3 {
            track.take(&mut out);
        }
        // 40 frames were left for the fourth take.
        track.take(&mut out);
        assert!(out[..40 * CHANNELS].iter().all(|&s| s == 0.5));
        assert!(out[40 * CHANNELS..].iter().all(|&s| s == 0.0));
        assert_eq!(track.stats.underruns, 1);
        assert_eq!(track.stats.silence, 440);
        // It waits for a cushion again.
        track.push(&frames(1000, 0.5));
        track.take(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn a_source_running_fast_loses_the_excess_in_one_go() {
        let mut track = TrackBuffer::default();
        track.push(&frames(MAX_QUEUED, 0.5));
        track.trim(0);
        assert_eq!(track.stats.dropped, 0);
        track.push(&frames(10, 0.5));
        track.trim(0);
        assert_eq!(track.stats.dropped, (MAX_QUEUED + 10 - CUSHION) as u64);
        assert_eq!(track.queued(), CUSHION);
        assert_eq!(track.stats.captured, (MAX_QUEUED + 10) as u64);
    }

    #[test]
    fn what_is_held_for_a_pause_being_caught_up_is_not_excess() {
        // Half a second without a wake-up: the device kept all of it, and
        // the clock will ask for all of it, a chunk at a time.
        let mut track = TrackBuffer::default();
        track.push(&frames(24_000 + CUSHION, 0.5));
        let mut out = frames(MAX_CHUNK, 0.0);
        let mut owed = 24_000;
        while owed > 0 {
            track.take(&mut out);
            owed -= MAX_CHUNK;
            track.trim(owed);
            assert!(out.iter().all(|&s| s == 0.5));
        }
        assert_eq!((track.stats.dropped, track.stats.silence, track.queued()), (0, 0, CUSHION));
    }

    #[test]
    fn the_tail_of_a_sound_is_not_played_late() {
        let mut track = TrackBuffer::default();
        track.push(&frames(100, 0.5));
        let mut out = frames(480, 9.0);
        for _ in 0..=STALE_TAKES {
            track.take(&mut out);
        }
        assert_eq!(track.stats.dropped, 100);
        track.push(&frames(480 + CUSHION, 0.25));
        track.take(&mut out);
        assert!(out.iter().all(|&s| s == 0.25), "only the new sound is heard");
    }

    /// Gives what a test sends it.
    struct Fed(mpsc::Receiver<Vec<f32>>, TrackBuffer);

    fn fed(from: mpsc::Receiver<Vec<f32>>) -> Box<Fed> {
        Box::new(Fed(from, TrackBuffer::default()))
    }

    impl Source for Fed {
        fn fill(&mut self, _now: Duration, out: &mut [f32], owed: usize) {
            while let Ok(samples) = self.0.try_recv() {
                self.1.push(&samples);
            }
            self.1.take(out);
            self.1.trim(owed);
        }
        fn stats(&self) -> TrackStats {
            self.1.stats
        }
        fn status(&self) -> String {
            "fed by a test".into()
        }
    }

    fn sample(block: &[u8], width: usize, frame: usize, channel: usize) -> f32 {
        let at = (frame * width + channel) * 4;
        f32::from_le_bytes(block[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn the_tracks_lie_side_by_side_in_order() {
        let (tx_a, rx_a) = mpsc::channel();
        let (tx_b, rx_b) = mpsc::channel();
        let mut mixer = Mixer::new(vec![fed(rx_a), fed(rx_b)]);
        tx_a.send(frames(480 + CUSHION, 0.25)).unwrap();
        tx_b.send(frames(480 + CUSHION, -0.5)).unwrap();
        let block = mixer.tick(ms(10)).to_vec();
        assert_eq!(block.len(), 480 * 4 * 4, "480 frames of four channels of four bytes");
        for frame in [0, 479] {
            assert_eq!(sample(&block, 4, frame, 0), 0.25);
            assert_eq!(sample(&block, 4, frame, 1), 0.25);
            assert_eq!(sample(&block, 4, frame, 2), -0.5);
            assert_eq!(sample(&block, 4, frame, 3), -0.5);
        }
    }

    #[test]
    fn a_dead_source_does_not_hold_the_other_track_or_the_clock() {
        let (tx, rx) = mpsc::channel();
        let (_never, silent) = mpsc::channel();
        let mut mixer = Mixer::new(vec![fed(silent), fed(rx)]);
        let mut bytes = 0;
        tx.send(frames(CUSHION, 0.5)).unwrap();
        for tick in 1..=100u64 {
            tx.send(frames(480, 0.5)).unwrap();
            bytes += mixer.tick(ms(tick * 10)).len();
        }
        assert_eq!(bytes, 48_000 * 4 * 4, "a second is a second's worth of bytes");
        let stats = mixer.stats();
        assert_eq!(stats[0], TrackStats { silence: 48_000, ..Default::default() });
        assert_eq!((stats[1].silence, stats[1].underruns, stats[1].dropped), (0, 0, 0));
    }

    #[test]
    fn a_source_a_little_fast_or_slow_drifts_into_the_counters_not_the_clock() {
        // 1% fast for a minute: the excess is dropped, the output is exact.
        for (per_tick, fast) in [(485usize, true), (475, false)] {
            let (tx, rx) = mpsc::channel();
            let mut mixer = Mixer::new(vec![fed(rx)]);
            let mut bytes = 0;
            for tick in 1..=6000u64 {
                tx.send(frames(per_tick, 0.5)).unwrap();
                bytes += mixer.tick(ms(tick * 10)).len();
            }
            assert_eq!(bytes, 60 * 48_000 * 2 * 4);
            let s = mixer.stats()[0];
            assert_eq!(s.captured, per_tick as u64 * 6000);
            if fast {
                assert!(s.dropped > 20_000 && s.dropped < 30_000, "{s:?}");
                assert_eq!(s.underruns, 0);
            } else {
                assert!(s.silence > 28_000 && s.silence < 34_000, "{s:?}");
                assert!(s.underruns > 10, "{s:?}");
                assert_eq!(s.dropped, 0);
            }
            // Every frame is accounted for: written, or still held.
            let held = s.captured + s.silence - s.dropped - 60 * 48_000;
            assert!(held <= MAX_QUEUED as u64, "{s:?}");
        }
    }

    #[test]
    fn with_no_tracks_there_is_nothing_to_write() {
        let mut mixer = Mixer::new(Vec::new());
        assert!(mixer.tick(ms(10)).is_empty());
        assert_eq!(mixer.report(ms(10)).len(), 1);
    }

    /// A pipe nobody reads from yet, as ffmpeg's is while it opens the screen.
    fn pipe() -> (ChildStdin, std::process::Child) {
        let mut reader = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "mixer::tests::reads_stdin_when_told_to", "--nocapture"])
            .env("ZENITH_TEST_READ_STDIN", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        (reader.stdin.take().unwrap(), reader)
    }

    /// The other end of [`pipe`]: this same test program, which waits
    /// half a second and then counts what it is sent.
    #[test]
    fn reads_stdin_when_told_to() {
        if std::env::var_os("ZENITH_TEST_READ_STDIN").is_none() {
            return;
        }
        std::thread::sleep(ms(500));
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes).unwrap();
        println!("read {} bytes, the first {}", bytes.len(), bytes.first().copied().unwrap_or(0));
    }

    #[test]
    fn a_reader_that_is_not_ready_never_holds_the_mixer_and_gets_the_sound_from_when_it_reads() {
        let (stdin, reader) = pipe();
        let mut sink = Sink::default();
        assert_eq!(sink.send(&[1; 16], 1 << 20), Sent::Nowhere);
        sink.connect(stdin);
        // Blocks as big as a second of four tracks: more than any pipe holds.
        let block = |n: u8| vec![n; 1 << 20];
        let began = Instant::now();
        assert_eq!(sink.send(&block(1), 4 << 20), Sent::Passed);
        let mut left_out = 0;
        let mut n = 1;
        while !sink.reading() {
            assert!(began.elapsed() < Duration::from_secs(20), "the reader never read");
            n += 1;
            if sink.send(&block(n), 4 << 20) == Sent::NotReadingYet {
                left_out += 1;
            }
            assert!(began.elapsed() < Duration::from_secs(20));
            std::thread::sleep(ms(10));
        }
        assert!(left_out > 10, "only one block waits for a reader that is not reading: {left_out} were left out");
        // What the pipe took before it read is made up for: as much again is left out.
        assert_eq!(sink.send(&block(199), 4 << 20), Sent::NotReadingYet);
        // Reading now: everything is passed on, up to the limit.
        assert_eq!(sink.send(&block(200), 4 << 20), Sent::Passed);
        assert_eq!(sink.send(&block(201), 1), Sent::Stuck);
        sink.disconnect();
        let out = reader.wait_with_output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert!(said.contains(", the first 1"), "{said}");
    }

    /// Stands in for ffmpeg's stdin: counts what it is given, and takes
    /// nothing while it is `held`.
    #[derive(Clone, Default)]
    struct Taker {
        held: Arc<AtomicBool>,
        got: Arc<AtomicUsize>,
        silent: Arc<AtomicUsize>,
    }

    impl Write for Taker {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            while self.held.load(Ordering::Relaxed) {
                std::thread::sleep(ms(1));
            }
            if bytes.iter().all(|&b| b == 0) {
                self.silent.fetch_add(bytes.len(), Ordering::Relaxed);
            }
            self.got.fetch_add(bytes.len(), Ordering::Relaxed);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A sink of one track whose ffmpeg is a [`Taker`] that reads already.
    fn taken(taker: &Taker) -> Sink {
        let mut sink = Sink::for_tracks(1);
        sink.connect(taker.clone());
        sink.writer.as_ref().unwrap().flow.reading.store(true, Ordering::Relaxed);
        sink
    }

    fn waiting(sink: &Sink) -> usize {
        sink.writer.as_ref().unwrap().flow.waiting.load(Ordering::Relaxed)
    }

    #[test]
    fn what_ffmpeg_did_not_take_while_it_was_stuck_is_made_up_with_silence() {
        let taker = Taker::default();
        let mut sink = taken(&taker);
        // 10 ms of one track at a wake-up; two seconds of it may wait.
        let block = vec![1u8; 480 * 8];
        let limit = 2 * 48_000 * 8;
        let mut offered = 0;
        let mut wake = |sink: &mut Sink| {
            offered += block.len();
            sink.send(&block, limit)
        };
        for _ in 0..100 {
            assert_eq!(wake(&mut sink), Sent::Passed);
        }
        while waiting(&sink) > 0 {
            std::thread::sleep(ms(1));
        }
        // Five seconds in which ffmpeg reads nothing: two of them wait for
        // it, the rest is left out.
        taker.held.store(true, Ordering::Relaxed);
        let stuck = (0..500).filter(|_| wake(&mut sink) == Sent::Stuck).count();
        assert!((299..=301).contains(&stuck), "{stuck} blocks were left out");
        assert!(sink.owed() >= stuck * block.len());
        assert!(waiting(&sink) <= limit, "what waits is never more than may");
        // It reads again: silence first, in pieces, and only then sound.
        taker.held.store(false, Ordering::Relaxed);
        let began = Instant::now();
        let mut catching_up = 0;
        loop {
            assert!(began.elapsed() < Duration::from_secs(20), "what was owed was never paid: {} bytes", sink.owed());
            let sent = wake(&mut sink);
            assert!(waiting(&sink) <= limit);
            // Left out again when the silence filled all the room there was.
            catching_up += usize::from(sent != Sent::Passed);
            std::thread::sleep(ms(1));
            if sent == Sent::Passed && sink.owed() == 0 && waiting(&sink) == 0 {
                break;
            }
        }
        assert!(catching_up > 0, "no sound is passed on before the silence it comes after");
        // Every byte the time asked for was written, as sound or as silence.
        assert_eq!(taker.got.load(Ordering::Relaxed), offered);
        assert_eq!(taker.silent.load(Ordering::Relaxed), (stuck + catching_up) * block.len());
        assert!(!sink.fell_behind());
    }

    #[test]
    fn time_the_clock_skipped_is_made_up_with_silence() {
        let taker = Taker::default();
        let sink = Mutex::new(taken(&taker));
        let (_never, silent) = mpsc::channel();
        let mut mixer = Mixer::new(vec![fed(silent)]);
        let limit = mixer.bytes_a_second() * MAX_WAITING_SECS;
        let mut now = 0;
        for _ in 0..100 {
            now += 10;
            assert_eq!(hand_over(&mut mixer, &sink, ms(now), limit), Sent::Passed);
        }
        // Five seconds without a wake-up (the machine slept): the mixer
        // makes no sound for them, and ffmpeg is owed them all the same.
        now += 5000;
        assert_eq!(hand_over(&mut mixer, &sink, ms(now), limit), Sent::CatchingUp);
        let began = Instant::now();
        loop {
            assert!(began.elapsed() < Duration::from_secs(20), "what was owed was never paid");
            now += 10;
            let sent = hand_over(&mut mixer, &sink, ms(now), limit);
            std::thread::sleep(ms(1));
            let sink = sink.lock().unwrap();
            if sent == Sent::Passed && sink.owed() == 0 && waiting(&sink) == 0 {
                break;
            }
        }
        // What was written is what the time since the start asks for.
        let written = taker.got.load(Ordering::Relaxed) / mixer.frame_bytes();
        let due = Pacer::target(ms(now)) as usize;
        assert!(written.abs_diff(due) <= 480, "{written} frames written, {due} due");
        assert!(mixer.pacer.skipped() > 4 * 48_000);
    }

    #[test]
    fn an_ffmpeg_that_has_not_begun_to_read_is_owed_nothing() {
        let taker = Taker::default();
        taker.held.store(true, Ordering::Relaxed);
        let mut sink = Sink::for_tracks(1);
        sink.connect(taker.clone());
        assert_eq!(sink.send(&[1; 3840], 1 << 20), Sent::Passed);
        assert_eq!(sink.send(&[1; 3840], 1 << 20), Sent::NotReadingYet);
        sink.owe(48_000 * 8);
        assert_eq!(sink.owed(), 0, "its sound starts when it does");
        // Nor is the next one owed what the last was.
        let mut sink = taken(&taker);
        assert_eq!(sink.send(&[1; 3840], 3840), Sent::Passed);
        assert_eq!(sink.send(&[1; 3840], 3840), Sent::Stuck);
        assert_eq!(sink.owed(), 3840);
        sink.connect(Taker::default());
        assert_eq!(sink.owed(), 0);
    }

    #[test]
    fn owing_more_than_can_be_made_up_for_is_told() {
        let taker = Taker::default();
        taker.held.store(true, Ordering::Relaxed);
        let mut sink = taken(&taker);
        let limit = 3840;
        sink.send(&[1; 3840], limit);
        for _ in 0..MAX_OWED {
            assert_eq!(sink.send(&[1; 3840], limit), Sent::Stuck);
            assert!(!sink.fell_behind());
        }
        assert_eq!(sink.send(&[1; 3840], limit), Sent::Stuck);
        assert!(sink.fell_behind(), "half a minute owed: that ffmpeg is started afresh");
        taker.held.store(false, Ordering::Relaxed);
    }

    #[test]
    fn a_reader_that_is_gone_is_let_go_of() {
        let (stdin, mut reader) = pipe();
        reader.kill().unwrap();
        reader.wait().unwrap();
        let mut sink = Sink::default();
        sink.connect(stdin);
        let began = Instant::now();
        while sink.send(&[0; 4096], 1 << 20) != Sent::Nowhere {
            assert!(began.elapsed() < Duration::from_secs(10), "the broken pipe was never noticed");
            std::thread::sleep(ms(5));
        }
    }
}
