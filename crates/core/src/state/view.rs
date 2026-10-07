//! What the key list currently shows: filtered, ordered, and pointing into the
//! arena (R2.2, R2.5, ADR-0010).
//!
//! Filtering and sorting never touch the [`LoadedSet`](super::LoadedSet). They
//! rebuild an index vector — `Vec<u32>` of positions into it — which is the
//! whole reason the store is columnar. A million-key sort permutes 4MB of
//! indices, not 40MB of names.

use super::loaded::LoadedSet;
use super::tree::NO_ROW;

/// How a filter pattern is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    /// Redis-style `*` and `?`, which is what users of `redis-cli` already know.
    #[default]
    Glob,
    /// Characters in order but not adjacent, for when you half-remember a name.
    Fuzzy,
}

/// Which column orders the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    /// Scan order — the order `SCAN` happened to return keys in. Not stable
    /// across scans, and honest about that: it is not called "natural".
    #[default]
    Scan,
    Name,
    Ttl,
    Size,
    Kind,
}

impl SortBy {
    pub fn label(&self) -> &'static str {
        match self {
            SortBy::Scan => "scan order",
            SortBy::Name => "name",
            SortBy::Ttl => "ttl",
            SortBy::Size => "size",
            SortBy::Kind => "type",
        }
    }

    /// Whether this column is fetched lazily, and so may be partly unknown.
    pub fn is_lazy(&self) -> bool {
        matches!(self, SortBy::Ttl | SortBy::Size | SortBy::Kind)
    }

    /// The cycle `s` walks through.
    pub fn next(&self) -> SortBy {
        match self {
            SortBy::Scan => SortBy::Name,
            SortBy::Name => SortBy::Ttl,
            SortBy::Ttl => SortBy::Size,
            SortBy::Size => SortBy::Kind,
            SortBy::Kind => SortBy::Scan,
        }
    }
}

/// What `order` was last built for. Narrowing is only sound against this, not
/// against the typed filter text, which can run ahead of it while a debounced
/// rebuild is pending (M4 task 4).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Applied {
    filter: String,
    mode: FilterMode,
    sort: SortBy,
    /// `keys.len()` when `order` was built or last extended: every key below
    /// this was offered to the filter, none at or above it was.
    covered: usize,
}

/// Whether every name matching `new` is guaranteed to match `prev`, so that
/// filtering the rows `prev` produced gives exactly what filtering the whole
/// Loaded set would.
///
/// Fuzzy is a subsequence test, so appending a character can only remove
/// matches. Glob is subtler: a pattern with no wildcard is a *substring*
/// search, and appending keeps the old text as a substring; a pattern ending
/// in `*` absorbs whatever follows. But a wildcard pattern that ends in a
/// literal is anchored at the end — `*a` matches `xa`, while its extension
/// `*ab` matches `xab`, which `*a` does not — so that case is not narrowable.
/// Proven exhaustively in `narrowing_is_sound_*` below.
fn narrows(mode: FilterMode, prev: &str, new: &str) -> bool {
    if !new.starts_with(prev) {
        return false;
    }
    match mode {
        FilterMode::Fuzzy => true,
        FilterMode::Glob => !prev.contains(['*', '?']) || prev.ends_with('*'),
    }
}

/// The ordered, filtered list of rows on offer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyView {
    order: Vec<u32>,
    /// Inverse of `order`: Loaded set index -> row, or `NO_ROW` when the
    /// index is filtered out. Dense, sized to the Loaded set as of the last
    /// `rebuild`/`extend`, and rewritten by exactly those two — the only
    /// places `order` is built or grown (M4 task 4, decision 5). Lives here
    /// rather than on `State` because it is `order`'s inverse: keeping it
    /// beside the field it mirrors means no caller can change one and forget
    /// the other.
    inverse: Vec<u32>,
    /// For a Name-sorted `order`: how many bytes each row's name shares with
    /// the row before it (0 for row 0), the sort's by-product (M6 task 5,
    /// `docs/plans/m6-fast-fold.md`). Parallel to `order`, so it lives and
    /// dies with it: set wherever a Name order is built or installed,
    /// patched by `narrow`, cleared by `extend`. Empty for any other sort.
    /// Read through [`KeyView::name_lcp`], which refuses it unless it is
    /// exactly as long as `order`.
    lcp: Vec<u32>,
    applied: Option<Applied>,
    pub filter: String,
    pub mode: FilterMode,
    pub sort: SortBy,
    /// How many of the shown rows have the sort column fetched (R2.5).
    known: usize,
}

impl KeyView {
    /// A view with a given filter and sort. The order is built by
    /// [`KeyView::rebuild`], never set directly.
    pub fn new(filter: impl Into<String>, mode: FilterMode, sort: SortBy) -> Self {
        Self {
            order: Vec::new(),
            inverse: Vec::new(),
            lcp: Vec::new(),
            applied: None,
            filter: filter.into(),
            mode,
            sort,
            known: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// The index vector itself, for a fold that reads it in place.
    pub(super) fn order(&self) -> &[u32] {
        &self.order
    }

    /// The LCP column for the fold, when the order is Name-sorted and the
    /// column is exactly as long as it. `None` makes the fold measure the
    /// shared prefixes itself.
    pub(super) fn name_lcp(&self) -> Option<&[u32]> {
        let name_sorted = self
            .applied
            .as_ref()
            .is_some_and(|a| a.sort == SortBy::Name);
        (name_sorted && self.lcp.len() == self.order.len()).then_some(self.lcp.as_slice())
    }

    /// How many Loaded set keys the order was built or extended over (zero
    /// before any build).
    pub fn covered_len(&self) -> usize {
        self.applied.as_ref().map_or(0, |a| a.covered)
    }

    /// Put a finished rebuild job's result in place: exactly the state
    /// [`KeyView::rebuild`] leaves, built for `filter`/`mode`/`sort` over
    /// `covered` keys (the job's snapshot, not the typed text).
    pub(super) fn install(
        &mut self,
        order: Vec<u32>,
        inverse: Vec<u32>,
        lcp: Vec<u32>,
        known: usize,
        built: (String, FilterMode, SortBy),
        covered: usize,
    ) {
        self.order = order;
        self.inverse = inverse;
        self.lcp = if built.2 == SortBy::Name {
            lcp
        } else {
            Vec::new()
        };
        self.known = known;
        self.applied = Some(Applied {
            filter: built.0,
            mode: built.1,
            sort: built.2,
            covered,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Heap bytes held by the order index alone (ADR-0010: "roughly 45MB of
    /// names plus ~15MB of indices at 1M keys" — this is the indices half,
    /// ~4MB/1M rows). Used by the M4 memory-budget test to account for the
    /// `View` alongside [`LoadedSet::heap_bytes`](super::loaded::LoadedSet::heap_bytes),
    /// which only covers the arena itself.
    pub fn heap_bytes(&self) -> usize {
        (self.order.capacity() + self.inverse.capacity() + self.lcp.capacity())
            * std::mem::size_of::<u32>()
    }

    /// The row a Loaded set index is shown at, if the filter lets it through.
    /// O(1) via the inverse index.
    pub fn row_of(&self, index: usize) -> Option<usize> {
        match self.inverse.get(index) {
            Some(&r) if r != NO_ROW => Some(r as usize),
            _ => None,
        }
    }

    /// Recompute `inverse` from `order` (after a sort permuted it).
    fn rebuild_inverse(&mut self, keys_len: usize) {
        self.inverse.clear();
        self.inverse.resize(keys_len, NO_ROW);
        for (row, &i) in self.order.iter().enumerate() {
            if let Some(slot) = self.inverse.get_mut(i as usize) {
                *slot = row as u32;
            }
        }
    }

    /// The Loaded set index shown at this row.
    pub fn index_at(&self, row: usize) -> Option<usize> {
        self.order.get(row).map(|i| *i as usize)
    }

    /// How many rows carry a value for the sort column.
    ///
    /// Sorting by a lazily-fetched column orders what has arrived and states
    /// this count; it never triggers a mass fetch (R2.5).
    pub fn known(&self) -> usize {
        self.known
    }

    /// What the filter line reads, e.g. `3,410 match`.
    pub fn match_readout(&self, total: usize) -> String {
        if self.filter.is_empty() {
            String::new()
        } else {
            format!("{} of {}", self.len(), total)
        }
    }

    /// What the sort readout says when the column is only partly known.
    pub fn sort_readout(&self) -> Option<String> {
        if self.sort == SortBy::Scan {
            return None;
        }
        if self.sort.is_lazy() && self.known < self.len() {
            Some(format!(
                "sorted by {} · {} of {} known",
                self.sort.label(),
                self.known,
                self.len()
            ))
        } else {
            Some(format!("sorted by {}", self.sort.label()))
        }
    }

    /// Recompute the order from the store. Cheap enough to run on every change.
    pub fn rebuild(&mut self, keys: &LoadedSet) {
        self.order.clear();
        self.order.reserve(keys.len());
        for i in 0..keys.len() {
            if self.filter.is_empty() {
                self.order.push(i as u32);
            } else if let Some(name) = keys.name(i)
                && matches(name, &self.filter, self.mode)
            {
                self.order.push(i as u32);
            }
        }
        self.lcp = self.apply_sort(keys);
        self.rebuild_inverse(keys.len());
        self.applied = Some(Applied {
            filter: self.filter.clone(),
            mode: self.mode,
            sort: self.sort,
            covered: keys.len(),
        });
    }

    /// Whether the order is exactly what [`KeyView::rebuild`] would produce
    /// right now, for the typed filter, mode and sort: built for those, over
    /// every one of `keys_len` loaded keys. Read-only; `Applied` stays private.
    ///
    /// Only meaningful for `Scan` and `Name` orders. A lazily-fetched sort
    /// re-sorts by whatever metadata has arrived since, so a rebuild can
    /// differ even when this is true; callers that need equivalence with a
    /// rebuild check the sort themselves (`State::view_is_current`).
    pub fn is_current_for(&self, keys_len: usize) -> bool {
        self.applied.as_ref().is_some_and(|a| {
            a.covered == keys_len
                && a.sort == self.sort
                && a.mode == self.mode
                && a.filter == self.filter
        })
    }

    /// Whether the typed filter can be applied by re-filtering the rows
    /// already shown instead of rebuilding from the Loaded set.
    ///
    /// Requires all of: the new text only removes matches ([`narrows`]); the
    /// same mode and sort the order was built under; the order covers every
    /// key currently loaded (a key that arrived since is in no row to keep);
    /// and a sort that does not depend on lazily-fetched metadata — a rebuild
    /// re-sorts by whatever has arrived since, so only `Scan` and `Name`
    /// orders are guaranteed identical either way.
    pub fn can_narrow(&self, keys_len: usize) -> bool {
        self.applied.as_ref().is_some_and(|a| {
            matches!(self.sort, SortBy::Scan | SortBy::Name)
                && a.sort == self.sort
                && a.mode == self.mode
                && a.covered == keys_len
                && narrows(a.mode, &a.filter, &self.filter)
        })
    }

    /// Re-filter the rows already shown against the typed filter. Only valid
    /// when [`KeyView::can_narrow`] said so. Rows keep their relative order,
    /// so the sort is preserved by construction; cost is the previous
    /// result's size, and the inverse index is patched, not rebuilt.
    pub fn narrow(&mut self, keys: &LoadedSet) {
        debug_assert!(self.can_narrow(keys.len()));
        let unchanged = self
            .applied
            .as_ref()
            .is_some_and(|a| a.filter == self.filter);
        if !unchanged {
            let mut kept = 0usize;
            // Two names' LCP is the minimum of the adjacent LCPs between
            // them, so dropping rows keeps the column exact: a kept row's
            // new LCP is the running minimum since the last kept row.
            let has_lcp = self.lcp.len() == self.order.len();
            let mut run = u32::MAX;
            for row in 0..self.order.len() {
                let i = self.order[row];
                if has_lcp {
                    run = run.min(self.lcp[row]);
                }
                let keep = self.filter.is_empty()
                    || keys
                        .name(i as usize)
                        .is_some_and(|name| matches(name, &self.filter, self.mode));
                if keep {
                    self.order[kept] = i;
                    if has_lcp {
                        self.lcp[kept] = if kept == 0 { 0 } else { run };
                        run = u32::MAX;
                    }
                    self.inverse[i as usize] = kept as u32;
                    kept += 1;
                } else {
                    self.inverse[i as usize] = NO_ROW;
                }
            }
            if has_lcp {
                self.lcp.truncate(kept);
            }
            self.order.truncate(kept);
        }
        self.known = self.order.len();
        if let Some(a) = &mut self.applied {
            a.filter = self.filter.clone();
        }
    }

    /// Append newly-scanned keys — indices `new_from..keys.len()` — without
    /// touching the rows already there (M4 task 3, `docs/plans/m4-perf-scan.md`
    /// decision 1).
    ///
    /// Correct **only** in scan order: appending preserves `SortBy::Scan`'s
    /// order exactly (a key arrives, it goes on the end, which is where scan
    /// order puts it), but would silently corrupt any other sort — a key
    /// sorted by name can belong anywhere in `order`, not at the tail. The
    /// caller (`scan_batch`) only takes this path when `sort == SortBy::Scan`;
    /// every other sort, and tree mode, goes through the batched
    /// [`KeyView::rebuild`] schedule instead.
    ///
    /// Cost is `O(new_from..keys.len())` — the page that just arrived — not
    /// `O(keys.len())`, which is what makes this the fix for the quadratic
    /// cost of calling [`KeyView::rebuild`] once per `ScanBatch`.
    pub fn extend(&mut self, keys: &LoadedSet, new_from: usize) {
        debug_assert_eq!(
            self.sort,
            SortBy::Scan,
            "extend is only correct in scan order; other sorts need a rebuild"
        );
        // Scan order has no LCP; a column left over from an earlier Name
        // order would no longer be parallel to `order`.
        self.lcp.clear();
        // Rows are only ever appended, so existing inverse entries stay
        // valid; the new tail starts as "filtered out" and is set below.
        self.inverse.resize(keys.len(), NO_ROW);
        // Filter by what `order` was built for, not by the typed text: while
        // a debounced rebuild is pending the two differ, and appending rows
        // under one filter to an order built under another would make the
        // eventual rebuild's answer depend on arrival timing.
        let applied = self.applied.get_or_insert_with(|| Applied {
            filter: self.filter.clone(),
            mode: self.mode,
            sort: self.sort,
            covered: new_from,
        });
        applied.covered = keys.len();
        let (filter, mode) = (applied.filter.as_str(), applied.mode);
        for i in new_from..keys.len() {
            if filter.is_empty() || keys.name(i).is_some_and(|name| matches(name, filter, mode)) {
                self.inverse[i] = self.order.len() as u32;
                self.order.push(i as u32);
            }
        }
        // Scan order has no lazily-fetched column to be partial about —
        // `known` tracks the whole order, same as `apply_sort`'s `Scan` arm.
        self.known = self.order.len();
    }

    /// Sort `order` by `sort`. Returns the Name sort's LCP column, empty for
    /// every other sort.
    fn apply_sort(&mut self, keys: &LoadedSet) -> Vec<u32> {
        self.known = match self.sort {
            SortBy::Scan | SortBy::Name => self.order.len(),
            SortBy::Ttl => self
                .order
                .iter()
                .filter(|i| keys.ttl(**i as usize).is_some())
                .count(),
            SortBy::Size => self
                .order
                .iter()
                .filter(|i| keys.size(**i as usize).is_some())
                .count(),
            SortBy::Kind => self
                .order
                .iter()
                .filter(|i| keys.kind(**i as usize).is_some())
                .count(),
        };

        match self.sort {
            SortBy::Scan => {}
            SortBy::Name => {
                // The record sort (M6 task 4): byte order of the names, equal
                // names in index order, which is what the stable `sort_by`
                // over an ascending order gave.
                return super::sort::sort_by_name(keys, &mut self.order);
            }
            // For every lazily-fetched column the rule is the same: order what
            // arrived, park the unknowns at the end in scan order, and say how
            // many were sorted. The alternative — firing a million commands
            // because somebody pressed a key — is the behaviour this project
            // exists to replace.
            SortBy::Ttl => self.sort_by_lazy(|i| keys.ttl(i).map(ttl_rank)),
            SortBy::Size => self.sort_by_lazy(|i| keys.size(i)),
            SortBy::Kind => self.sort_by_lazy(|i| keys.kind(i).map(|k| k as u8)),
        }
        Vec::new()
    }

    fn sort_by_lazy<T: Ord, F: Fn(usize) -> Option<T>>(&mut self, value: F) {
        self.order.sort_by(|a, b| {
            match (value(*a as usize), value(*b as usize)) {
                (Some(x), Some(y)) => x.cmp(&y).then(a.cmp(b)),
                // Unknown sorts last, and keeps scan order among itself so the
                // tail does not shuffle every time one more value arrives.
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(b),
            }
        });
    }
}

/// The one comparison every sort uses when it cannot run as a single
/// `sort_by`: the rebuild job's chunk sort and merge (M6 task 3). Same
/// semantics as [`KeyView::apply_sort`]: `Name` is the byte order of the
/// names; each lazy column orders what is known by value then index, with
/// unknowns last in scan order. `Scan` is index order.
pub(super) fn order_cmp(keys: &LoadedSet, sort: SortBy, a: u32, b: u32) -> std::cmp::Ordering {
    fn lazy<T: Ord>(x: Option<T>, y: Option<T>, a: u32, b: u32) -> std::cmp::Ordering {
        match (x, y) {
            (Some(x), Some(y)) => x.cmp(&y).then(a.cmp(&b)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(&b),
        }
    }
    let (ia, ib) = (a as usize, b as usize);
    match sort {
        SortBy::Scan => a.cmp(&b),
        SortBy::Name => keys.name(ia).cmp(&keys.name(ib)),
        SortBy::Ttl => lazy(keys.ttl(ia).map(ttl_rank), keys.ttl(ib).map(ttl_rank), a, b),
        SortBy::Size => lazy(keys.size(ia), keys.size(ib), a, b),
        SortBy::Kind => lazy(
            keys.kind(ia).map(|k| k as u8),
            keys.kind(ib).map(|k| k as u8),
            a,
            b,
        ),
    }
}

/// Whether a key has a value for a lazy sort column (always true for the
/// others): what `known` counts.
pub(super) fn has_sort_value(keys: &LoadedSet, sort: SortBy, i: usize) -> bool {
    match sort {
        SortBy::Scan | SortBy::Name => true,
        SortBy::Ttl => keys.ttl(i).is_some(),
        SortBy::Size => keys.size(i).is_some(),
        SortBy::Kind => keys.kind(i).is_some(),
    }
}

/// A key with no expiry sorts after every key that has one: `∞` is the largest
/// TTL there is, not a missing value.
fn ttl_rank(seconds: i32) -> i64 {
    if seconds == super::loaded::TTL_NONE {
        i64::MAX
    } else {
        seconds as i64
    }
}

/// Whether a key name matches a pattern.
pub fn matches(name: &[u8], pattern: &str, mode: FilterMode) -> bool {
    match mode {
        FilterMode::Glob => glob(name, pattern.as_bytes()),
        FilterMode::Fuzzy => fuzzy(name, pattern.as_bytes()),
    }
}

/// Redis-style glob: `*` any run, `?` any single byte.
///
/// A pattern with no wildcards is treated as a substring search, because that
/// is what people mean when they type `session` into a filter box.
fn glob(name: &[u8], pattern: &[u8]) -> bool {
    if !pattern.contains(&b'*') && !pattern.contains(&b'?') {
        return name
            .windows(pattern.len().max(1))
            .any(|w| w.eq_ignore_ascii_case(pattern))
            || pattern.is_empty();
    }
    glob_at(name, pattern)
}

fn glob_at(name: &[u8], pattern: &[u8]) -> bool {
    let (mut n, mut p) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == b'?' || eq_ci(pattern[p], name[n])) {
            n += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = p;
            mark = n;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            n = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

/// Characters in order, not necessarily adjacent.
fn fuzzy(name: &[u8], pattern: &[u8]) -> bool {
    let mut it = name.iter();
    pattern.iter().all(|c| it.any(|n| eq_ci(*c, *n)))
}

fn eq_ci(a: u8, b: u8) -> bool {
    a.eq_ignore_ascii_case(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(names: &[&str]) -> LoadedSet {
        let mut s = LoadedSet::default();
        for n in names {
            s.push(n.as_bytes());
        }
        s
    }

    fn viewed(keys: &LoadedSet, view: &KeyView) -> Vec<String> {
        (0..view.len())
            .map(|r| {
                keys.name_str(view.index_at(r).unwrap())
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    // ── filtering (M1.5) ────────────────────────────────────────────────────

    #[test]
    fn a_glob_filters_the_way_redis_cli_users_expect() {
        let keys = store(&[
            "user:1:session",
            "user:2:session",
            "user:1:cart",
            "feed:hot",
        ]);
        let mut v = KeyView {
            filter: "user:*:session".into(),
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["user:1:session", "user:2:session"]);
    }

    #[test]
    fn a_bare_word_is_a_substring_search_because_that_is_what_people_mean() {
        let keys = store(&["user:1:session", "user:1:cart", "feed:hot"]);
        let mut v = KeyView {
            filter: "cart".into(),
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["user:1:cart"]);
    }

    #[test]
    fn question_mark_matches_exactly_one() {
        let keys = store(&["k:1", "k:12", "k:2"]);
        let mut v = KeyView {
            filter: "k:?".into(),
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["k:1", "k:2"]);
    }

    #[test]
    fn fuzzy_finds_a_half_remembered_name() {
        let keys = store(&["user:8812:session", "feed:global:hot", "cart:91af"]);
        let mut v = KeyView {
            filter: "usession".into(),
            mode: FilterMode::Fuzzy,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["user:8812:session"]);
    }

    #[test]
    fn an_empty_filter_shows_everything() {
        let keys = store(&["a", "b", "c"]);
        let mut v = KeyView::default();
        v.rebuild(&keys);
        assert_eq!(v.len(), 3);
        assert_eq!(v.match_readout(3), "", "no filter, so nothing to report");
    }

    #[test]
    fn the_match_count_is_stated_against_the_whole_loaded_set() {
        let keys = store(&["user:1", "user:2", "feed:1"]);
        let mut v = KeyView {
            filter: "user:*".into(),
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(v.match_readout(keys.len()), "2 of 3");
    }

    #[test]
    fn filtering_leaves_the_store_untouched() {
        // ADR-0010: filtering permutes an index vector, never the arena.
        let keys = store(&["a", "b", "c"]);
        let before = keys.clone();
        let mut v = KeyView {
            filter: "a".into(),
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(keys, before);
    }

    // ── sorting (M1.7) ──────────────────────────────────────────────────────

    #[test]
    fn sorting_by_name_is_never_partial_because_names_are_never_lazy() {
        let keys = store(&["c", "a", "b"]);
        let mut v = KeyView {
            sort: SortBy::Name,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["a", "b", "c"]);
        assert_eq!(v.sort_readout().as_deref(), Some("sorted by name"));
    }

    #[test]
    fn sorting_by_size_orders_what_arrived_and_parks_the_rest() {
        let mut keys = store(&["big", "small", "unknown-a", "unknown-b"]);
        keys.set_size(0, 5_000);
        keys.set_size(1, 100);
        let mut v = KeyView {
            sort: SortBy::Size,
            ..KeyView::default()
        };
        v.rebuild(&keys);

        assert_eq!(
            viewed(&keys, &v),
            ["small", "big", "unknown-a", "unknown-b"],
            "known values sort; unknowns go to the end in scan order"
        );
        assert_eq!(v.known(), 2);
        assert_eq!(
            v.sort_readout().as_deref(),
            Some("sorted by size · 2 of 4 known"),
            "the reader must be told the sort is partial"
        );
    }

    #[test]
    fn a_key_with_no_expiry_sorts_after_every_key_that_has_one() {
        // ∞ is the largest TTL there is, not a missing value.
        let mut keys = store(&["forever", "soon", "later"]);
        keys.set_ttl(0, super::super::loaded::TTL_NONE, 0);
        keys.set_ttl(1, 30, 0);
        keys.set_ttl(2, 3_600, 0);
        let mut v = KeyView {
            sort: SortBy::Ttl,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["soon", "later", "forever"]);
        assert_eq!(v.known(), 3, "no expiry is a known fact, not a gap");
    }

    /// Sort-by-TTL must not reshuffle rows once a second as their countdowns
    /// cross each other — that would look broken, not live. `sort_by_lazy`
    /// reads the raw, unmoving `ttl()`, never `ttl_now()`; this pins that as a
    /// property of the sort rather than an accident of which function nobody
    /// happened to call.
    #[test]
    fn sort_by_ttl_does_not_reshuffle_as_the_clock_advances() {
        let mut keys = store(&["soon", "later"]);
        keys.set_ttl(0, 10, 0);
        keys.set_ttl(1, 20, 0);
        let mut v = KeyView {
            sort: SortBy::Ttl,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        let order = viewed(&keys, &v);
        assert_eq!(order, ["soon", "later"]);

        // 15 seconds on: "soon"'s displayed countdown would now read behind
        // "later"'s original number, but nothing here has been re-sorted —
        // rebuild wasn't even called again.
        assert_eq!(keys.ttl_now(0, 15), Some(0));
        assert_eq!(keys.ttl_now(1, 15), Some(5));
        assert_eq!(
            viewed(&keys, &v),
            order,
            "the clock alone must not change sort order"
        );
    }

    #[test]
    fn the_unknown_tail_does_not_shuffle_as_values_trickle_in() {
        // Otherwise the list would churn under the reader while metadata loads.
        let mut keys = store(&["a", "b", "c", "d"]);
        keys.set_size(0, 10);
        let mut v = KeyView {
            sort: SortBy::Size,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        let tail_before = viewed(&keys, &v)[1..].to_vec();

        keys.set_size(3, 5);
        v.rebuild(&keys);
        let after = viewed(&keys, &v);
        assert_eq!(&after[..2], ["d", "a"], "the two known sizes sort");
        assert_eq!(
            &after[2..],
            &tail_before[..2],
            "the unknown tail kept its order"
        );
    }

    #[test]
    fn sorting_never_asks_for_anything() {
        // The proof for R2.5 is a negative: this function has no way to fetch.
        // It takes an immutable store and returns an ordering, so a mass fetch
        // is not merely avoided, it is unrepresentable.
        let keys = store(&["a", "b"]);
        let before = keys.clone();
        let mut v = KeyView {
            sort: SortBy::Size,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(keys, before);
        assert_eq!(v.known(), 0);
    }

    #[test]
    fn the_sort_cycle_returns_to_where_it_started() {
        let mut s = SortBy::Scan;
        for _ in 0..5 {
            s = s.next();
        }
        assert_eq!(s, SortBy::Scan);
    }

    #[test]
    fn scan_order_is_named_honestly() {
        // SCAN guarantees no ordering, so calling this "natural" would be a lie.
        assert_eq!(SortBy::Scan.label(), "scan order");
        assert_eq!(KeyView::default().sort_readout(), None);
    }

    // ── incremental append during a scan (M4 task 3) ───────────────────────

    #[test]
    fn extend_matches_a_full_rebuild_in_scan_order() {
        let mut keys = store(&["a", "b", "c"]);
        let mut extended = KeyView::default();
        extended.rebuild(&keys);

        keys.push(b"d");
        keys.push(b"e");
        extended.extend(&keys, 3);

        let mut rebuilt = KeyView::default();
        rebuilt.rebuild(&keys);

        assert_eq!(viewed(&keys, &extended), viewed(&keys, &rebuilt));
        assert_eq!(viewed(&keys, &extended), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn extend_applies_the_filter_to_only_the_new_keys() {
        let mut keys = store(&["user:1", "feed:1"]);
        let mut v = KeyView {
            filter: "user:*".into(),
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["user:1"]);

        keys.push(b"user:2");
        keys.push(b"feed:2");
        v.extend(&keys, 2);

        let mut rebuilt = KeyView {
            filter: "user:*".into(),
            ..KeyView::default()
        };
        rebuilt.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), viewed(&keys, &rebuilt));
        assert_eq!(viewed(&keys, &v), ["user:1", "user:2"]);
    }

    #[test]
    fn extend_over_several_pages_matches_one_rebuild_at_the_end() {
        let mut keys = LoadedSet::default();
        let mut v = KeyView::default();
        for page in 0..10 {
            for i in page * 10..page * 10 + 10 {
                keys.push(format!("k:{i}").as_bytes());
            }
            let prior = page * 10;
            v.extend(&keys, prior);
        }
        let mut rebuilt = KeyView::default();
        rebuilt.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), viewed(&keys, &rebuilt));
        assert_eq!(v.len(), 100);
    }

    #[test]
    fn filter_and_sort_compose() {
        let mut keys = store(&["user:b", "user:a", "feed:z"]);
        keys.set_size(0, 200);
        keys.set_size(1, 100);
        let mut v = KeyView {
            filter: "user:*".into(),
            sort: SortBy::Size,
            ..KeyView::default()
        };
        v.rebuild(&keys);
        assert_eq!(viewed(&keys, &v), ["user:a", "user:b"]);
    }

    // ── inverse index (M4 task 4) ──────────────────────────────────────────

    fn assert_inverse_matches_linear(keys: &LoadedSet, v: &KeyView) {
        for index in 0..keys.len() + 3 {
            let linear = (0..v.len()).find(|&r| v.index_at(r) == Some(index));
            assert_eq!(v.row_of(index), linear, "row_of({index})");
        }
    }

    #[test]
    fn row_of_matches_a_linear_scan_under_every_sort_and_filter() {
        let mut keys = LoadedSet::default();
        for i in 0..200 {
            keys.push(format!("k{}:{}", (i * 7) % 13, i).as_bytes());
        }
        for i in (0..200).step_by(3) {
            keys.set_size(i, ((i * 31) % 17) as u32);
        }
        for sort in [SortBy::Scan, SortBy::Name, SortBy::Size, SortBy::Kind] {
            for filter in ["", "k1*", "zzz", "5"] {
                let mut v = KeyView::new(filter, FilterMode::Glob, sort);
                v.rebuild(&keys);
                assert_inverse_matches_linear(&keys, &v);
            }
        }
    }

    #[test]
    fn row_of_stays_correct_through_extend_and_equals_a_rebuilt_view() {
        for filter in ["", "k1*"] {
            let mut keys = LoadedSet::default();
            let mut v = KeyView::new(filter, FilterMode::Glob, SortBy::Scan);
            v.rebuild(&keys);
            for page in 0..8 {
                let prior = keys.len();
                for i in 0..25 {
                    keys.push(format!("k{}:{}", (i + page) % 4, page * 25 + i).as_bytes());
                }
                v.extend(&keys, prior);
                assert_inverse_matches_linear(&keys, &v);
                let mut rebuilt = KeyView::new(filter, FilterMode::Glob, SortBy::Scan);
                rebuilt.rebuild(&keys);
                assert_eq!(v, rebuilt, "extended view must equal a rebuilt one");
            }
        }
    }

    // ── narrowing soundness (M4 task 4, decision 1) ────────────────────────

    /// Every pattern over a small alphabet, `0..=max_len` long.
    fn all_strings(alphabet: &[char], max_len: usize) -> Vec<String> {
        let mut out = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..max_len {
            let mut next = Vec::new();
            for s in &frontier {
                for c in alphabet {
                    let mut t = s.clone();
                    t.push(*c);
                    next.push(t);
                }
            }
            out.extend(next.iter().cloned());
            frontier = next;
        }
        out
    }

    fn assert_narrowing_sound(mode: FilterMode, pattern_alphabet: &[char]) {
        let patterns = all_strings(pattern_alphabet, 4);
        let names = all_strings(&['a', 'b', 'A', ':'], 5);
        for prev in &patterns {
            for new in &patterns {
                if !narrows(mode, prev, new) {
                    continue;
                }
                for n in &names {
                    assert!(
                        !matches(n.as_bytes(), new, mode) || matches(n.as_bytes(), prev, mode),
                        "{mode:?}: {n:?} matches {new:?} but not {prev:?}, which narrows() allowed"
                    );
                }
            }
        }
    }

    #[test]
    fn narrowing_is_sound_for_fuzzy() {
        assert_narrowing_sound(FilterMode::Fuzzy, &['a', 'b', 'A', ':']);
    }

    #[test]
    fn narrowing_is_sound_for_glob_with_wildcards() {
        assert_narrowing_sound(FilterMode::Glob, &['a', 'b', '*', '?']);
    }

    #[test]
    fn glob_anchored_at_the_end_is_the_case_that_must_not_narrow() {
        // The reason glob is not "extends => narrows": `*a` matches `xa`,
        // its extension `*ab` matches `xab`, and `xab` is not a `*a` match.
        assert!(matches(b"xab", "*ab", FilterMode::Glob));
        assert!(!matches(b"xab", "*a", FilterMode::Glob));
        assert!(!narrows(FilterMode::Glob, "*a", "*ab"));
        assert!(!narrows(FilterMode::Glob, "a?", "a?b"));
        assert!(narrows(FilterMode::Glob, "ab", "abc"), "substring: safe");
        assert!(narrows(FilterMode::Glob, "a*", "a*b"), "trailing *: safe");
        assert!(
            narrows(FilterMode::Fuzzy, "a?", "a?b"),
            "fuzzy: always safe"
        );
    }
}
