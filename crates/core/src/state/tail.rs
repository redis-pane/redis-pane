//! `LiveTail<T>` — the cap-bounded, pause-aware buffer every live-tail view
//! shares (Monitor today, Pub/Sub from `docs/plans/m3-pubsub.md`).
//!
//! Extracted out of `MonitorState` (`docs/plans/m3-pubsub.md` decision 10:
//! "share the tail, don't copy it") once Pub/Sub needed the exact same
//! discipline — a bounded `VecDeque`, pause that stops *consuming* rather
//! than just hiding, a following selection, and a display-only filter
//! string — so the two features cannot drift apart on the one thing that
//! matters most here (ADR-0010's cap discipline, generalized from a scan to
//! a stream, applied twice).
//!
//! `LiveTail` owns none of a feature's per-item cap: each feature keeps its
//! own named constant (`MONITOR_CAP`, `PUBSUB_CAP`) and passes it to
//! [`LiveTail::push`] — the cap is still enforced in exactly one function
//! *per feature* (`push_monitor_line`, `push_pubsub_message`), which
//! delegate to this one shared mechanism rather than each reimplementing
//! eviction.

use std::collections::VecDeque;

/// A live, capped tail of `T`, with pause/following/selection/filter —
/// everything about *how a tail is browsed* that does not depend on what
/// `T` is. What differs per feature (the cap, how an item is truncated
/// before it is pushed, how it is matched against `filter`) stays in the
/// feature's own state module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTail<T> {
    items: VecDeque<T>,
    /// Pause stops *consuming*, not just hiding (the Monitor decision this
    /// generalizes: "a paused Monitor is not a queue, it is a closed
    /// valve"). Left `pub` because both features' state types are
    /// constructed and asserted against directly in unit tests the way
    /// `MonitorState` already was.
    pub paused: bool,
    /// Items that arrived while paused — counted, never buffered. Reset to
    /// `0` on resume, so each pause reports its own count.
    pub dropped_while_paused: u64,
    /// The display-only filter string. Narrows what's *shown*, never what's
    /// buffered — matching semantics belong to the feature (glob over raw
    /// text for Monitor, channel-or-payload substring for Pub/Sub), not
    /// here.
    pub filter: String,
    selected: usize,
    /// Whether the tail is following new items as they arrive. Any move off
    /// the last item turns this off without pausing consumption;
    /// `to_bottom` turns it back on.
    pub following: bool,
}

impl<T> Default for LiveTail<T> {
    // Not `#[derive(Default)]`: that would add a spurious `T: Default`
    // bound (`VecDeque<T>` itself needs none), and every feature's item
    // type has no reason to implement `Default`.
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
            paused: false,
            dropped_while_paused: 0,
            filter: String::new(),
            selected: 0,
            following: false,
        }
    }
}

impl<T> LiveTail<T> {
    pub fn items(&self) -> &VecDeque<T> {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.items.get(self.selected)
    }

    /// Start fresh: empties the buffer, clears pause/filter/drop-count, and
    /// resumes following. A feature's own `reset()` calls this and then
    /// restores whatever it does not want cleared (Monitor keeps
    /// `feed_token`; both keep `status`).
    pub fn reset(&mut self) {
        *self = LiveTail::default();
    }

    /// The one function that appends — every feature's `push_*` delegates
    /// here rather than touching `items` directly (ADR-0010's "enforced in
    /// exactly one place", generalized). `cap` is supplied by the caller,
    /// not stored, since a bare tail has no opinion of its own about how
    /// big it should be allowed to grow.
    ///
    /// Paused: the item is not appended at all — counted in
    /// `dropped_while_paused` and nothing else. This is what makes pause
    /// stop *consuming*, not merely *hiding*: `items` does not grow while
    /// paused, regardless of how many items the feed delivers underneath.
    pub fn push(&mut self, item: T, cap: usize) {
        if self.paused {
            self.dropped_while_paused += 1;
            return;
        }
        self.items.push_back(item);
        if self.items.len() > cap {
            self.items.pop_front();
            // A position, not a stable id: eviction shifts every later item
            // down by one, and the item at `selected` shifts with it. `0`
            // clamps to the new oldest item; it does not move further,
            // since the item it names is still there.
            self.selected = self.selected.saturating_sub(1);
        }
        if self.following {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// Movement within the tail. Any move away from the last item stops
    /// following — resuming is `to_bottom`'s job alone.
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        self.following = false;
        let max = (self.items.len() - 1) as isize;
        let new = (self.selected as isize + delta).clamp(0, max);
        self.selected = new as usize;
    }

    pub fn to_top(&mut self) {
        self.following = false;
        self.selected = 0;
    }

    /// Jump to the newest item and resume following.
    pub fn to_bottom(&mut self) {
        self.following = true;
        self.selected = self.items.len().saturating_sub(1);
    }

    /// Flip pause. Resuming clears the drop count — a fresh count for the
    /// next pause, not a running session total.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.dropped_while_paused = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_never_exceeded_over_a_long_synthetic_run() {
        let mut t: LiveTail<u64> = LiveTail::default();
        let cap = 50;
        for i in 0..(cap * 3) {
            t.push(i as u64, cap);
            assert!(t.len() <= cap, "grew past the cap at {i}");
        }
        assert_eq!(t.len(), cap);
        assert_eq!(*t.items().front().unwrap(), (cap * 3 - cap) as u64);
    }

    #[test]
    fn pausing_counts_items_rather_than_buffering_them() {
        let mut t: LiveTail<u64> = LiveTail::default();
        t.push(1, 100);
        t.toggle_pause();
        assert!(t.paused);
        for i in 0..10 {
            t.push(i, 100);
        }
        assert_eq!(t.len(), 1, "the buffer never grew while paused");
        assert_eq!(t.dropped_while_paused, 10);
    }

    #[test]
    fn resuming_does_not_backfill_what_was_dropped() {
        let mut t: LiveTail<u64> = LiveTail::default();
        t.toggle_pause();
        t.push(1, 100);
        assert_eq!(t.dropped_while_paused, 1);
        t.toggle_pause(); // resume
        assert!(!t.paused);
        assert_eq!(t.len(), 0);
        assert_eq!(t.dropped_while_paused, 0);
        t.push(2, 100);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn following_tracks_new_items_until_moved_off_the_end() {
        let mut t: LiveTail<u64> = LiveTail {
            following: true,
            ..LiveTail::default()
        };
        for i in 0..5 {
            t.push(i, 100);
        }
        assert_eq!(t.selected(), 4);
        t.move_selection(-1);
        assert!(!t.following);
        assert_eq!(t.selected(), 3);
        t.push(5, 100);
        assert_eq!(t.selected(), 3);
        t.to_bottom();
        assert!(t.following);
        assert_eq!(t.selected(), 5);
    }

    #[test]
    fn eviction_clamps_a_selection_on_the_oldest_item_rather_than_panicking() {
        let mut t: LiveTail<u64> = LiveTail::default();
        for i in 0..3 {
            t.push(i, 3);
        }
        t.to_top();
        assert_eq!(t.selected(), 0);
        for i in 0..100 {
            t.push(100 + i, 3);
        }
        assert_eq!(t.selected(), 0);
    }

    #[test]
    fn reset_clears_everything() {
        let mut t: LiveTail<u64> = LiveTail::default();
        t.push(1, 100);
        t.paused = true;
        t.filter = "x".into();
        t.following = false;
        t.reset();
        assert!(t.is_empty());
        assert!(!t.paused);
        assert!(t.filter.is_empty());
        assert!(!t.following, "a bare reset does not opt into following");
    }
}
