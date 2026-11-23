//! The line of work waiting for a few worker threads: pictures to make for
//! the screens. What someone is looking at right now goes first, nothing is
//! made twice, and what nobody is looking at any more is not made at all.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

/// How long what could not be made is left alone before it may be asked
/// for again. A file that cannot be read now cannot be read four seconds
/// from now either, but a clip that was still being written, or on a drive
/// that was busy, can be a little later: its card should not stay blank
/// until the app is started again.
pub const RETRY_AFTER: Duration = Duration::from_secs(30);

/// Jobs waiting to be done, in two lines: the urgent one is emptied first.
pub struct Jobs<J> {
    urgent: VecDeque<J>,
    rest: VecDeque<J>,
    /// What has been asked for and is not made yet, so it is made once.
    pending: HashSet<J>,
    /// What could not be made, and when that was. It is not tried again
    /// until `RETRY_AFTER` has passed.
    failed: HashMap<J, Instant>,
}

impl<J> Default for Jobs<J> {
    fn default() -> Self {
        Self { urgent: VecDeque::new(), rest: VecDeque::new(), pending: HashSet::new(), failed: HashMap::new() }
    }
}

impl<J: Clone + Eq + Hash> Jobs<J> {
    /// Ask for a job. `false` when it is waiting or being done already, or
    /// failed a moment ago: there is nothing new for a worker then.
    pub fn ask(&mut self, job: J, urgent: bool) -> bool {
        self.ask_at(job, urgent, Instant::now())
    }

    /// `ask`, at the time `now`.
    pub fn ask_at(&mut self, job: J, urgent: bool, now: Instant) -> bool {
        if self.failed.get(&job).is_some_and(|at| now.saturating_duration_since(*at) < RETRY_AFTER) || !self.pending.insert(job.clone()) {
            return false;
        }
        self.failed.remove(&job);
        if urgent { self.urgent.push_back(job) } else { self.rest.push_back(job) }
        true
    }

    /// Take back the waiting jobs `unwanted` picks out: the card that asked
    /// has scrolled out of sight, so a card still in sight is not kept
    /// waiting behind it. A job a worker has begun is finished. Returns how
    /// many were taken back.
    pub fn withdraw(&mut self, unwanted: impl Fn(&J) -> bool) -> usize {
        let before = self.rest.len();
        let pending = &mut self.pending;
        self.rest.retain(|job| {
            let drop = unwanted(job);
            if drop {
                pending.remove(job);
            }
            !drop
        });
        before - self.rest.len()
    }

    /// The next job for a worker. It stays pending while it is made, so a
    /// second request for it waits for this one.
    pub fn take(&mut self) -> Option<J> {
        self.urgent.pop_front().or_else(|| self.rest.pop_front())
    }

    /// A worker is done with `job`, well or not.
    pub fn done(&mut self, job: J, failed: bool) {
        self.done_at(job, failed, Instant::now());
    }

    /// `done`, at the time `now`.
    pub fn done_at(&mut self, job: J, failed: bool, now: Instant) {
        self.pending.remove(&job);
        if failed {
            // Failures nobody asked about again are not kept for ever.
            self.failed.retain(|_, at| now.saturating_duration_since(*at) < RETRY_AFTER);
            self.failed.insert(job, now);
        }
    }

    /// Let what failed be asked for again now, for the jobs `changed`
    /// picks out: their file is another one than the one that failed.
    pub fn forgive(&mut self, changed: impl Fn(&J) -> bool) {
        self.failed.retain(|job, _| !changed(job));
    }

    /// How many jobs wait for a worker.
    pub fn waiting(&self) -> usize {
        self.urgent.len() + self.rest.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgent_jobs_go_first_and_nothing_is_made_twice() {
        let mut jobs = Jobs::default();
        assert!(jobs.ask("thumb a", false));
        assert!(jobs.ask("thumb b", false));
        assert!(jobs.ask("filmstrip", true));
        assert!(!jobs.ask("thumb a", false), "already waiting");
        assert_eq!(jobs.take(), Some("filmstrip"));
        assert_eq!(jobs.take(), Some("thumb a"));
        assert!(!jobs.ask("thumb a", false), "being made");
        jobs.done("thumb a", false);
        assert!(jobs.ask("thumb a", false), "made and asked for again (its file was removed)");
        jobs.done("filmstrip", true);
        assert!(!jobs.ask("filmstrip", true), "what failed is not tried again at once");
        assert_eq!(jobs.waiting(), 2);
    }

    #[test]
    fn what_failed_is_tried_again_after_a_while_or_when_its_file_changed() {
        let mut jobs = Jobs::default();
        let start = Instant::now();
        let after = |secs: u64| start + Duration::from_secs(secs);
        assert!(jobs.ask_at("thumb a", false, start));
        assert_eq!(jobs.take(), Some("thumb a"));
        jobs.done_at("thumb a", true, after(1));
        assert!(!jobs.ask_at("thumb a", false, after(5)), "the card asks every few seconds");
        assert!(!jobs.ask_at("thumb a", false, after(30)));
        assert!(jobs.ask_at("thumb a", false, after(31)), "a while later it is worth another look");
        assert!(!jobs.ask_at("thumb a", false, after(32)), "and that look is one job, not two");
        assert_eq!(jobs.take(), Some("thumb a"));
        // It fails again: the wait starts again from then.
        jobs.done_at("thumb a", true, after(40));
        assert!(!jobs.ask_at("thumb a", false, after(60)));
        // Its file was written again meanwhile.
        jobs.forgive(|job| *job == "thumb a");
        assert!(jobs.ask_at("thumb a", false, after(61)));
        // What failed long ago and was never asked for again is not kept.
        jobs.done_at("thumb b", true, after(100));
        jobs.done_at("thumb c", true, after(200));
        assert_eq!(jobs.failed.len(), 1);
    }

    #[test]
    fn what_scrolled_out_of_sight_is_not_made() {
        let mut jobs = Jobs::default();
        // A long library flicked through: forty cards asked, then left.
        for card in 0..40 {
            jobs.ask(card, false);
        }
        // A worker has the first one in hand already.
        assert_eq!(jobs.take(), Some(0));
        assert_eq!(jobs.withdraw(|card| *card < 36), 35);
        assert_eq!(jobs.waiting(), 4, "only the cards on screen are left");
        assert_eq!(jobs.take(), Some(36));
        // The one in hand is finished; one taken back can be asked again.
        assert!(!jobs.ask(0, false));
        assert!(jobs.ask(5, false));
        // Urgent work is never taken back this way.
        jobs.ask(100, true);
        assert_eq!(jobs.withdraw(|_| true), 4);
        assert_eq!(jobs.take(), Some(100));
        assert_eq!(jobs.take(), None);
    }
}
