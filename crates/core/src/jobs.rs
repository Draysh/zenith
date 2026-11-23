//! The line of work waiting for a few worker threads: pictures to make for
//! the screens. What someone is looking at right now goes first, nothing is
//! made twice, and what nobody is looking at any more is not made at all.

use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

/// Jobs waiting to be done, in two lines: the urgent one is emptied first.
pub struct Jobs<J> {
    urgent: VecDeque<J>,
    rest: VecDeque<J>,
    /// What has been asked for and is not made yet, so it is made once.
    pending: HashSet<J>,
    /// What could not be made. It is not tried again: a file that cannot
    /// be read now cannot be read four seconds from now either.
    failed: HashSet<J>,
}

impl<J> Default for Jobs<J> {
    fn default() -> Self {
        Self { urgent: VecDeque::new(), rest: VecDeque::new(), pending: HashSet::new(), failed: HashSet::new() }
    }
}

impl<J: Clone + Eq + Hash> Jobs<J> {
    /// Ask for a job. `false` when it is waiting or being done already, or
    /// failed before: there is nothing new for a worker then.
    pub fn ask(&mut self, job: J, urgent: bool) -> bool {
        if self.failed.contains(&job) || !self.pending.insert(job.clone()) {
            return false;
        }
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
        self.pending.remove(&job);
        if failed {
            self.failed.insert(job);
        }
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
        assert!(!jobs.ask("filmstrip", true), "what failed is not tried again");
        assert_eq!(jobs.waiting(), 2);
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
