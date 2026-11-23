//! An edit of a clip: which stretches of it are kept, and how loud each
//! audio track is. The clip's file is never changed; an edit is applied
//! when it is previewed and when it is exported.

use serde::{Deserialize, Serialize};

/// The shortest stretch that can be kept, in seconds.
pub const MIN_SEGMENT: f64 = 0.1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

impl Segment {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }

    pub fn contains(&self, t: f64) -> bool {
        t >= self.start && t < self.end
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Gain {
    /// 1.0 is as recorded; 0 to 2.
    pub volume: f64,
    pub muted: bool,
}

impl Default for Gain {
    fn default() -> Self {
        Self { volume: 1.0, muted: false }
    }
}

impl Gain {
    /// The factor the track is multiplied by.
    pub fn factor(&self) -> f64 {
        if self.muted { 0.0 } else { self.volume.clamp(0.0, 2.0) }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Edit {
    /// The clip's length in seconds.
    pub duration: f64,
    /// The kept stretches, in order, never touching or overlapping.
    pub segments: Vec<Segment>,
    /// One per audio track of the clip.
    pub gains: Vec<Gain>,
    /// How much later (or, below zero, earlier) this clip's sound is put
    /// against its picture, in milliseconds.
    #[serde(default)]
    pub audio_offset_ms: i32,
}

/// The furthest a clip's sound may be moved against its picture, either way.
pub const MAX_AUDIO_OFFSET_MS: i32 = 3000;

/// How far an edit's length may be from its file's and still be an edit of
/// that file: two readings of one file differ by a frame or so.
pub const LENGTH_SLACK: f64 = 0.5;

impl Edit {
    /// The whole clip, as recorded.
    pub fn new(duration: f64, audio_tracks: usize) -> Self {
        let duration = duration.max(MIN_SEGMENT);
        Self { duration, segments: vec![Segment { start: 0.0, end: duration }], gains: vec![Gain::default(); audio_tracks], audio_offset_ms: 0 }
    }

    /// An edit as it was kept, made fit to work with for a file of
    /// `seconds`. What is kept is not checked when it is written and may be
    /// from another Zenith, or about a file that has since been replaced:
    /// an edit of another length than the file is dropped (the clip is one
    /// not edited yet), and stretches that are out of order, outside the
    /// clip or too short are put right or left out, so that everything else
    /// here can count on what the fields say. An edit that was made here
    /// comes back as it is.
    pub fn fitted(mut self, seconds: f64) -> Option<Self> {
        let numbers = self.duration.is_finite() && self.segments.iter().all(|s| s.start.is_finite() && s.end.is_finite());
        if !numbers || self.duration <= 0.0 || (self.duration - seconds).abs() >= LENGTH_SLACK {
            return None;
        }
        let mut from = 0.0;
        let mut kept = Vec::with_capacity(self.segments.len());
        for s in &self.segments {
            let (start, end) = (s.start.max(from), s.end.min(self.duration));
            // The same hair of slack the trims give a stretch.
            if end - start >= MIN_SEGMENT - 1e-9 {
                kept.push(Segment { start, end });
                from = end;
            }
        }
        if kept.is_empty() {
            // Never empty: the whole clip, with the volumes it had.
            kept.push(Segment { start: 0.0, end: self.duration });
        }
        self.segments = kept;
        for gain in &mut self.gains {
            gain.volume = if gain.volume.is_finite() { gain.volume.clamp(0.0, 2.0) } else { 1.0 };
        }
        self.audio_offset_ms = self.audio_offset_ms.clamp(-MAX_AUDIO_OFFSET_MS, MAX_AUDIO_OFFSET_MS);
        Some(self)
    }

    /// Whether any of the clip is cut away: a trimmed end or a removed
    /// part. (A split alone cuts nothing away.)
    pub fn is_trimmed(&self) -> bool {
        (self.kept() - self.duration).abs() > 1e-6
    }

    /// Move the sound against the picture.
    pub fn set_audio_offset(&mut self, ms: i32) {
        self.audio_offset_ms = ms.clamp(-MAX_AUDIO_OFFSET_MS, MAX_AUDIO_OFFSET_MS);
    }

    /// Whether anything differs from the clip as recorded.
    pub fn is_changed(&self) -> bool {
        *self != Self::new(self.duration, self.gains.len())
    }

    /// How long the result is.
    pub fn kept(&self) -> f64 {
        self.segments.iter().map(Segment::len).sum()
    }

    /// The kept stretch that holds `t`.
    pub fn segment_at(&self, t: f64) -> Option<usize> {
        self.segments.iter().position(|s| s.contains(t))
    }

    /// Cut the stretch that holds `t` in two there. Returns whether it did:
    /// not in a removed part, and not so close to an edge that a piece
    /// would be shorter than `MIN_SEGMENT`.
    pub fn split(&mut self, t: f64) -> bool {
        let Some(i) = self.segment_at(t) else { return false };
        let s = self.segments[i];
        if t - s.start < MIN_SEGMENT || s.end - t < MIN_SEGMENT {
            return false;
        }
        self.segments[i].end = t;
        self.segments.insert(i + 1, Segment { start: t, end: s.end });
        true
    }

    /// Drop a kept stretch. The last one stays: an edit is never empty.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.segments.len() || self.segments.len() == 1 {
            return false;
        }
        self.segments.remove(index);
        true
    }

    /// Move a stretch's start, within the room it has (the end of the one
    /// before it, its own end). Returns where it landed.
    pub fn set_start(&mut self, index: usize, t: f64) -> Option<f64> {
        let s = *self.segments.get(index)?;
        let low = if index == 0 { 0.0 } else { self.segments[index - 1].end };
        let t = t.clamp(low, (s.end - MIN_SEGMENT).max(low));
        self.segments[index].start = t;
        Some(t)
    }

    /// Move a stretch's end, within the room it has.
    pub fn set_end(&mut self, index: usize, t: f64) -> Option<f64> {
        let s = *self.segments.get(index)?;
        let high = self.segments.get(index + 1).map_or(self.duration, |n| n.start);
        let t = t.clamp((s.start + MIN_SEGMENT).min(high), high);
        self.segments[index].end = t;
        Some(t)
    }

    /// Trim the front: the result starts at `t`.
    pub fn trim_start(&mut self, t: f64) {
        // Not `clamp`, which panics when its bounds cross (a clip shorter
        // than the shortest stretch): release builds abort on a panic.
        let t = t.min(self.segments.last().map_or(0.0, |s| s.end - MIN_SEGMENT)).max(0.0);
        // A hair of slack: 10.0 - 9.9 is a little under 0.1 in floating point.
        self.segments.retain(|s| s.end - t >= MIN_SEGMENT - 1e-9);
        if let Some(first) = self.segments.first_mut() {
            first.start = first.start.max(t);
        }
    }

    /// Trim the back: the result ends at `t`.
    pub fn trim_end(&mut self, t: f64) {
        let t = t.max(self.segments.first().map_or(0.0, |s| s.start + MIN_SEGMENT)).min(self.duration);
        self.segments.retain(|s| t - s.start >= MIN_SEGMENT - 1e-9);
        if let Some(last) = self.segments.last_mut() {
            last.end = last.end.min(t);
        }
    }

    /// Where playback at clip time `t` should be: `t` itself inside a kept
    /// stretch, the start of the next one inside a removed part, `None`
    /// past the end of the last.
    pub fn playable(&self, t: f64) -> Option<f64> {
        self.segments.iter().find(|s| t < s.end).map(|s| t.max(s.start))
    }

    /// A time in the clip as a time in the result.
    pub fn to_output(&self, t: f64) -> f64 {
        let mut out = 0.0;
        for s in &self.segments {
            if t < s.start {
                break;
            }
            out += t.min(s.end) - s.start;
        }
        out
    }

    /// A time in the result as a time in the clip.
    pub fn to_source(&self, out: f64) -> f64 {
        let mut left = out.max(0.0);
        for s in &self.segments {
            if left <= s.len() {
                return s.start + left;
            }
            left -= s.len();
        }
        self.segments.last().map_or(0.0, |s| s.end)
    }

    pub fn set_volume(&mut self, track: usize, volume: f64) {
        if let Some(g) = self.gains.get_mut(track) {
            g.volume = volume.clamp(0.0, 2.0);
        }
    }

    pub fn set_muted(&mut self, track: usize, muted: bool) {
        if let Some(g) = self.gains.get_mut(track) {
            g.muted = muted;
        }
    }

    /// Whether the result has any sound.
    pub fn audible(&self) -> bool {
        self.gains.iter().any(|g| g.factor() > 0.0)
    }
}

/// Undo and redo over the states of something being edited.
#[derive(Clone, Debug)]
pub struct History<T> {
    past: Vec<T>,
    now: T,
    future: Vec<T>,
}

impl<T: Clone + PartialEq> History<T> {
    const LIMIT: usize = 200;

    pub fn new(start: T) -> Self {
        Self { past: Vec::new(), now: start, future: Vec::new() }
    }

    pub fn now(&self) -> &T {
        &self.now
    }

    /// Record a new state. Nothing is recorded when it equals the current one.
    pub fn push(&mut self, next: T) {
        if next == self.now {
            return;
        }
        self.past.push(std::mem::replace(&mut self.now, next));
        if self.past.len() > Self::LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// Replace the current state without a step (while a handle is dragged).
    pub fn amend(&mut self, next: T) {
        self.now = next;
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.past.pop() else { return false };
        self.future.push(std::mem::replace(&mut self.now, prev));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.future.pop() else { return false };
        self.past.push(std::mem::replace(&mut self.now, next));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(e: &Edit) -> Vec<(f64, f64)> {
        e.segments.iter().map(|s| (s.start, s.end)).collect()
    }

    #[test]
    fn a_new_edit_is_the_whole_clip() {
        let e = Edit::new(60.0, 3);
        assert_eq!(spans(&e), [(0.0, 60.0)]);
        assert!(!e.is_changed());
        assert_eq!(e.kept(), 60.0);
        assert!(e.audible());
    }

    #[test]
    fn splitting_and_removing_cuts_out_the_middle() {
        let mut e = Edit::new(60.0, 1);
        assert!(e.split(10.0));
        assert!(e.split(20.0));
        assert!(e.remove(1));
        assert_eq!(spans(&e), [(0.0, 10.0), (20.0, 60.0)]);
        assert_eq!(e.kept(), 50.0);
        assert!(e.is_changed());
        assert!(!e.split(15.0), "nothing to split in a removed part");
        assert!(!e.split(20.05), "too close to an edge");
        assert!(e.remove(0));
        assert!(!e.remove(0), "the last stretch stays");
    }

    #[test]
    fn handles_stay_inside_their_room() {
        let mut e = Edit::new(60.0, 1);
        e.split(10.0);
        e.split(20.0);
        e.remove(1);
        // The second stretch's start cannot pass the first one's end.
        assert_eq!(e.set_start(1, 5.0), Some(10.0));
        assert_eq!(e.set_start(1, 25.0), Some(25.0));
        assert_eq!(e.set_start(1, 100.0), Some(60.0 - MIN_SEGMENT));
        assert_eq!(e.set_end(0, 30.0), Some(30.0));
        assert_eq!(e.set_end(0, 100.0), Some(60.0 - MIN_SEGMENT), "up to the next stretch's start");
        assert_eq!(e.set_end(0, -5.0), Some(MIN_SEGMENT));
        assert_eq!(e.set_end(1, 999.0), Some(60.0));
        assert_eq!(e.set_end(9, 1.0), None);
    }

    #[test]
    fn trims_drop_what_falls_outside() {
        let mut e = Edit::new(60.0, 1);
        e.split(10.0);
        e.split(20.0);
        e.remove(1);
        e.trim_start(15.0);
        assert_eq!(spans(&e), [(20.0, 60.0)]);
        e.trim_start(30.0);
        e.trim_end(50.0);
        assert_eq!(spans(&e), [(30.0, 50.0)]);
        e.trim_end(0.0);
        assert_eq!(spans(&e), [(30.0, 30.0 + MIN_SEGMENT)], "never empty");
    }

    #[test]
    fn a_trim_to_the_very_end_or_start_leaves_something() {
        for duration in [0.5, 1.0, 10.0, 15.5, 59.94, 60.0, 120.0, 599.9] {
            let mut front = Edit::new(duration, 1);
            front.trim_start(duration);
            assert_eq!(front.segments.len(), 1, "front of {duration}");
            assert!(front.kept() > 0.0);
            let mut back = Edit::new(duration, 1);
            back.trim_end(0.0);
            assert_eq!(back.segments.len(), 1, "back of {duration}");
            assert!(back.kept() > 0.0);
        }
    }

    #[test]
    fn times_map_between_clip_and_result() {
        let mut e = Edit::new(60.0, 1);
        e.split(10.0);
        e.split(20.0);
        e.remove(1);
        assert_eq!(e.to_output(5.0), 5.0);
        assert_eq!(e.to_output(15.0), 10.0, "inside the cut");
        assert_eq!(e.to_output(25.0), 15.0);
        assert_eq!(e.to_source(15.0), 25.0);
        assert_eq!(e.to_source(999.0), 60.0);
        assert_eq!(e.playable(5.0), Some(5.0));
        assert_eq!(e.playable(12.0), Some(20.0), "playback jumps the cut");
        assert_eq!(e.playable(60.0), None);
    }

    #[test]
    fn gains_mute_and_clamp() {
        let mut e = Edit::new(10.0, 2);
        e.set_volume(0, 5.0);
        e.set_muted(1, true);
        e.set_volume(7, 1.0);
        assert_eq!(e.gains[0].factor(), 2.0);
        assert_eq!(e.gains[1].factor(), 0.0);
        assert!(e.audible());
        e.set_muted(0, true);
        assert!(!e.audible());
    }

    #[test]
    fn only_what_is_cut_away_counts_as_trimmed() {
        let mut e = Edit::new(60.0, 2);
        e.set_volume(0, 0.3);
        e.set_muted(1, true);
        e.set_audio_offset(5000);
        assert_eq!(e.audio_offset_ms, MAX_AUDIO_OFFSET_MS);
        e.split(30.0);
        assert!(e.is_changed());
        assert!(!e.is_trimmed(), "volumes, sync and a split cut nothing away");
        e.set_end(1, 50.0);
        assert!(e.is_trimmed());
        e.set_end(1, 60.0);
        assert!(!e.is_trimmed(), "put back, it is whole again");
    }

    #[test]
    fn an_edit_made_here_is_fit_as_it_is() {
        let mut e = Edit::new(60.0, 2);
        e.split(10.0);
        e.split(20.0);
        e.remove(1);
        e.trim_start(5.0);
        e.trim_end(0.0);
        e.set_volume(1, 0.3);
        e.set_audio_offset(-250);
        assert_eq!(e.clone().fitted(60.0), Some(e.clone()));
        assert_eq!(e.clone().fitted(60.3), Some(e.clone()), "two readings of one file");
        assert_eq!(Edit::new(0.04, 1).fitted(0.04), Some(Edit::new(0.04, 1)), "a clip shorter than the shortest stretch");
        // The file is another one now, longer or shorter.
        assert_eq!(e.clone().fitted(30.0), None);
        assert_eq!(e.fitted(90.0), None);
    }

    #[test]
    fn an_edit_that_was_kept_wrong_is_put_right_and_never_panics() {
        let read = |json: &str| serde_json::from_str::<Edit>(json).unwrap();
        // Out of order, overlapping, outside the clip, too short, backwards.
        let odd = read(
            r#"{"duration":10.0,"segments":[{"start":-3.0,"end":2.0},{"start":1.0,"end":4.0},{"start":4.0,"end":4.01},{"start":9.0,"end":7.0},{"start":8.0,"end":99.0}],"gains":[{"volume":9.0,"muted":false}],"audio_offset_ms":90000}"#,
        );
        let e = odd.fitted(10.0).unwrap();
        assert_eq!(spans(&e), [(0.0, 2.0), (2.0, 4.0), (8.0, 10.0)]);
        assert_eq!((e.gains[0].volume, e.audio_offset_ms), (2.0, MAX_AUDIO_OFFSET_MS));
        // Nothing usable in it: the whole clip.
        let none = read(r#"{"duration":10.0,"segments":[{"start":50.0,"end":60.0}],"gains":[]}"#);
        assert_eq!(spans(&none.fitted(10.0).unwrap()), [(0.0, 10.0)]);
        assert_eq!(read(r#"{"duration":-4.0,"segments":[],"gains":[]}"#).fitted(-4.0), None);

        // Not fitted at all, every way to change it is still safe to call.
        for json in [
            r#"{"duration":0.05,"segments":[{"start":0.0,"end":0.05}],"gains":[]}"#,
            r#"{"duration":1.0,"segments":[{"start":5.0,"end":2.0}],"gains":[]}"#,
            r#"{"duration":-1.0,"segments":[{"start":0.95,"end":1.0}],"gains":[]}"#,
            r#"{"duration":10.0,"segments":[],"gains":[]}"#,
        ] {
            for t in [-1.0, 0.0, 0.5, 100.0, f64::NAN] {
                let mut e = read(json);
                e.trim_start(t);
                e.trim_end(t);
                e.set_start(0, t);
                e.set_end(0, t);
                e.split(t);
                let mut e = read(json);
                e.trim_end(t);
                e.trim_start(t);
            }
        }
    }

    #[test]
    fn an_edit_from_before_the_sync_still_reads() {
        let old = r#"{"duration":10.0,"segments":[{"start":0.0,"end":10.0}],"gains":[{"volume":1.0,"muted":false}]}"#;
        let e: Edit = serde_json::from_str(old).unwrap();
        assert_eq!(e, Edit::new(10.0, 1));
    }

    #[test]
    fn it_round_trips_as_json() {
        let mut e = Edit::new(42.5, 3);
        e.split(10.0);
        e.set_volume(2, 0.5);
        e.set_audio_offset(-120);
        let back: Edit = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn history_undoes_and_redoes() {
        let mut h = History::new(1);
        h.push(2);
        h.push(2);
        h.push(3);
        assert!(h.undo());
        assert_eq!(*h.now(), 2);
        assert!(h.redo());
        assert_eq!(*h.now(), 3);
        h.undo();
        h.undo();
        assert!(!h.undo());
        h.push(9);
        assert!(!h.can_redo(), "a new step drops what was undone");
        h.amend(10);
        assert!(h.undo());
        assert_eq!(*h.now(), 1);
    }
}
