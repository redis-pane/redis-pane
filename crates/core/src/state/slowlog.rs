//! Slowlog state: the `g s` view's copy of the server's own ring buffer of
//! slow commands (R6.4, `docs/plans/m3-slowlog.md`).
//!
//! A plain `Vec`, not the keys pane's byte-arena-plus-parallel-arrays
//! treatment (ADR-0010): that discipline exists because the keyspace can
//! reach a million keys, and `slowlog-max-len` bounds this ring buffer at a
//! few hundred entries by construction — a plain `Vec<SlowlogEntry>` is the
//! right amount of machinery here, not a small copy of the keys pane's arena
//! for its own sake. Sort still permutes an index vector, never the backing
//! `Vec` — the same idiom [`crate::state::view::KeyView`]'s sort uses, so a
//! reader of this code recognises it, not because this list is ever large
//! enough to need it.

/// One entry from `SLOWLOG GET`: `(id, timestamp, duration_us, args,
/// client_addr, client_name)` — the six-element shape every server at this
/// app's Redis 6.0 floor returns (ADR-0007); the pre-4.0 four-element shape
/// is out of reach. `command` is the args already joined with a single
/// space into one byte string, shell-side (`crates/app/src/redis/read.rs`) —
/// cheaper to carry than `Vec<Vec<u8>>` and exactly what the detail strip
/// and a copied command both want; nothing here ever needs one argument
/// pulled back out from the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlowlogEntry {
    /// The node this entry came from (`host:port`) on a Cluster, empty
    /// otherwise. Entry ids are unique only per node, so a row's identity is
    /// `(node, id)` (M5 task 8, ADR-0022).
    pub node: String,
    pub id: i64,
    /// Unix seconds, as the server reports it.
    pub timestamp: i64,
    pub duration_us: i64,
    pub command: Vec<u8>,
    pub client_addr: Vec<u8>,
    pub client_name: Vec<u8>,
}

impl SlowlogEntry {
    /// What names this row across a refetch: ids restart per node.
    pub fn identity(&self) -> (&str, i64) {
        (&self.node, self.id)
    }
}

/// One node whose `SLOWLOG GET` failed (M5 task 8): the view still shows every
/// other node's entries and names this one, with the server's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFailure {
    pub node: String,
    pub detail: String,
}

/// Which column orders the list (the same idiom `state::view::SortBy` uses
/// for the keys pane, one type over).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlowlogSort {
    /// Most recent first: by timestamp, and entries from one second keep the
    /// order they arrived in (`SLOWLOG GET`'s own newest-first order, node by
    /// node on a Cluster).
    #[default]
    Recent,
    /// Slowest first — the column this screen exists to let someone sort by.
    Duration,
}

impl SlowlogSort {
    pub fn label(&self) -> &'static str {
        match self {
            SlowlogSort::Recent => "recent",
            SlowlogSort::Duration => "duration",
        }
    }

    /// The cycle `s` walks through.
    pub fn next(&self) -> SlowlogSort {
        match self {
            SlowlogSort::Recent => SlowlogSort::Duration,
            SlowlogSort::Duration => SlowlogSort::Recent,
        }
    }
}

/// The Slowlog view's whole state (`View::Slowlog`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlowlogState {
    entries: Vec<SlowlogEntry>,
    /// A permutation of `0..entries.len()` — sorting reorders this, never
    /// `entries` itself (ADR-0010's idiom, one type over).
    order: Vec<u32>,
    pub sort: SlowlogSort,
    pub selected: usize,
    /// A fetch is in flight — set by `g s`/`r`, cleared by
    /// `Msg::SlowlogLoaded`/`Msg::SlowlogFailed`.
    pub loading: bool,
    /// The last fetch's failure, if the most recent one failed — the
    /// in-view empty state's own words, alongside the R7.4 notification
    /// `State::error` separately carries.
    pub error: Option<String>,
    /// Nodes that did not answer the last fetch (Cluster only, M5 task 8).
    pub failed: Vec<NodeFailure>,
}

impl SlowlogState {
    pub fn entries(&self) -> &[SlowlogEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Replace the whole set after a fetch, and rebuild the sort order over
    /// it. The selection follows the entry it was on, by `(node, id)`, when
    /// that entry is still in the set; otherwise it clamps to the new length
    /// (`SLOWLOG RESET` and the ring buffer's own eviction both retire an id
    /// for good).
    pub fn set_entries(&mut self, entries: Vec<SlowlogEntry>) {
        let followed = self.selected_entry().map(|e| (e.node.clone(), e.id));
        self.entries = entries;
        self.rebuild_order();
        self.selected = self.selected.min(self.order.len().saturating_sub(1));
        if let Some((node, id)) = followed
            && let Some(pos) = self
                .order
                .iter()
                .position(|&i| self.entries[i as usize].identity() == (node.as_str(), id))
        {
            self.selected = pos;
        }
    }

    fn rebuild_order(&mut self) {
        let mut order: Vec<u32> = (0..self.entries.len() as u32).collect();
        let entries = &self.entries;
        match self.sort {
            // A stable sort: on one node this is `SLOWLOG GET`'s own order
            // (newest first), and across nodes it interleaves them by time.
            SlowlogSort::Recent => order.sort_by(|a, b| {
                entries[*b as usize]
                    .timestamp
                    .cmp(&entries[*a as usize].timestamp)
            }),
            SlowlogSort::Duration => order.sort_by(|a, b| {
                entries[*b as usize]
                    .duration_us
                    .cmp(&entries[*a as usize].duration_us)
            }),
        }
        self.order = order;
    }

    pub fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.rebuild_order();
    }

    pub fn selected_entry(&self) -> Option<&SlowlogEntry> {
        self.order
            .get(self.selected)
            .and_then(|&i| self.entries.get(i as usize))
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.order.is_empty() {
            return;
        }
        let max = self.order.len() - 1;
        let new = (self.selected as isize + delta).clamp(0, max as isize);
        self.selected = new as usize;
    }

    pub fn top(&mut self) {
        self.selected = 0;
    }

    pub fn bottom(&mut self) {
        self.selected = self.order.len().saturating_sub(1);
    }

    /// The entries in their current sort order, for rendering.
    pub fn ordered(&self) -> impl Iterator<Item = &SlowlogEntry> {
        self.order.iter().map(move |&i| &self.entries[i as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i64, duration_us: i64) -> SlowlogEntry {
        SlowlogEntry {
            node: String::new(),
            id,
            timestamp: 1_700_000_000 + id,
            duration_us,
            command: format!("GET k{id}").into_bytes(),
            client_addr: b"127.0.0.1:1234".to_vec(),
            client_name: Vec::new(),
        }
    }

    #[test]
    fn recent_is_the_default_and_keeps_the_servers_own_order() {
        let mut s = SlowlogState::default();
        s.set_entries(vec![entry(3, 100), entry(2, 500), entry(1, 50)]);
        let ids: Vec<i64> = s.ordered().map(|e| e.id).collect();
        assert_eq!(ids, vec![3, 2, 1]);
    }

    #[test]
    fn duration_sort_permutes_the_index_never_the_backing_vec() {
        let mut s = SlowlogState::default();
        s.set_entries(vec![entry(3, 100), entry(2, 500), entry(1, 50)]);
        let before = s.entries().to_vec();
        s.cycle_sort();
        assert_eq!(s.sort, SlowlogSort::Duration);
        let ids: Vec<i64> = s.ordered().map(|e| e.id).collect();
        assert_eq!(ids, vec![2, 3, 1], "slowest first");
        assert_eq!(s.entries().to_vec(), before, "the backing Vec never moved");
    }

    #[test]
    fn cycling_sort_twice_returns_to_recent() {
        let mut s = SlowlogState::default();
        s.set_entries(vec![entry(1, 10), entry(2, 20)]);
        s.cycle_sort();
        s.cycle_sort();
        assert_eq!(s.sort, SlowlogSort::Recent);
    }

    #[test]
    fn selection_clamps_to_the_new_length_on_a_smaller_fetch() {
        let mut s = SlowlogState::default();
        s.set_entries(vec![entry(1, 10), entry(2, 20), entry(3, 30)]);
        s.selected = 2;
        s.set_entries(vec![entry(4, 10)]);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn move_selection_clamps_at_both_ends() {
        let mut s = SlowlogState::default();
        s.set_entries(vec![entry(1, 10), entry(2, 20)]);
        s.move_selection(-5);
        assert_eq!(s.selected, 0);
        s.move_selection(5);
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn move_selection_on_an_empty_set_does_not_panic() {
        let mut s = SlowlogState::default();
        s.move_selection(1);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn selected_entry_follows_the_sort_order_not_the_backing_vec() {
        let mut s = SlowlogState::default();
        s.set_entries(vec![entry(3, 100), entry(2, 500), entry(1, 50)]);
        s.cycle_sort(); // Duration: 2 (500), 3 (100), 1 (50)
        assert_eq!(s.selected_entry().unwrap().id, 2);
    }
}
