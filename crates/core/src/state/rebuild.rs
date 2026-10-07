//! The sliced rebuild job (M6 task 3, `docs/plans/m6-rebuild-job.md`).
//!
//! A rebuild of a million-key list is hundreds of milliseconds of filter,
//! sort and fold. The core is pure and owns no clock, so it cannot run that on
//! a thread or stop itself after so many milliseconds: instead the rebuild is
//! a *value*, a [`RebuildJob`], that the shell steps forward one bounded slice
//! at a time (`Msg::RebuildStep`, answered to `Command::ContinueRebuild`).
//! A slice is a count of keys ([`REBUILD_SLICE`]), calibrated by the perf
//! suite, never a duration.
//!
//! The job builds into buffers of its own. `State::list`, `State::tree` and
//! `State::tree_mode` keep describing the *shown* list, fully navigable,
//! until the swap replaces them in one `update`.

use std::cmp::Ordering;

use super::tree::{FoldProgress, NO_ROW};
use super::view::{has_sort_value, matches, order_cmp};
use super::{FilterMode, KeyView, LoadedSet, SortBy, State, Tree};

/// Keys (or output positions) one step of a rebuild job handles.
///
/// Calibrated so the slowest step, a fold step over random-order deep names
/// at 1M keys, stays comfortably inside a 16ms frame on CI
/// (`docs/plans/m6-rebuild-job.md` Outcome).
pub const REBUILD_SLICE: usize = 32_768;

/// What a job rebuilds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// Filter, sort, inverse, then fold when tree mode is the target.
    Full,
    /// Only the fold, over the shown flat view (`State::fold_is_current`).
    FoldOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Filter,
    Sort,
    Inverse,
    Fold,
}

/// Where the chunk sort and bottom-up merge are.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SortPhase {
    /// Sorting `slice`-sized chunks of `order` in place; `next` is the start
    /// of the next chunk.
    Chunks { next: usize },
    /// Merging runs of `width` from `order` into `scratch`, resumable
    /// mid-pair.
    Merge(Merge),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Merge {
    width: usize,
    /// Output position within the pass.
    out: usize,
    /// The pair being merged: left run `lo..mid`, right run `mid..hi`, with
    /// read cursors `i` and `j`.
    mid: usize,
    hi: usize,
    i: usize,
    j: usize,
}

impl Merge {
    fn pass(width: usize, n: usize) -> Merge {
        Merge {
            width,
            out: 0,
            mid: width.min(n),
            hi: (2 * width).min(n),
            i: 0,
            j: width.min(n),
        }
    }
}

/// A rebuild in progress: the target, a stage and cursor, and the partial
/// result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebuildJob {
    kind: JobKind,
    stage: Stage,
    /// What the job is building (a snapshot at the start).
    filter: String,
    mode: FilterMode,
    sort: SortBy,
    want_tree: bool,
    /// `keys.len()` when the job started: every key below it is offered to
    /// the filter, none at or above.
    covered: usize,
    slice: usize,
    /// Scan pages have arrived since the job started.
    dirty: bool,
    // Filter stage.
    cursor: usize,
    order: Vec<u32>,
    known: usize,
    // Sort stage.
    scratch: Vec<u32>,
    sort_phase: SortPhase,
    // Inverse stage.
    inverse: Vec<u32>,
    // Fold stage.
    tree: Option<Tree>,
    fold: FoldProgress,
}

impl RebuildJob {
    /// A job that filters, sorts and (when `want_tree`) folds every one of
    /// `covered` keys. `sort` must already be `Name` when `want_tree`.
    pub(super) fn full(
        filter: String,
        mode: FilterMode,
        sort: SortBy,
        want_tree: bool,
        covered: usize,
        slice: usize,
        tree: &Tree,
    ) -> RebuildJob {
        RebuildJob {
            kind: JobKind::Full,
            stage: Stage::Filter,
            filter,
            mode,
            sort,
            want_tree,
            covered,
            slice: slice.max(1),
            dirty: false,
            cursor: 0,
            order: Vec::with_capacity(covered),
            known: 0,
            scratch: Vec::new(),
            sort_phase: SortPhase::Chunks { next: 0 },
            inverse: Vec::new(),
            tree: want_tree.then(|| tree.shell()),
            fold: FoldProgress::default(),
        }
    }

    /// A job that only folds the shown flat view.
    pub(super) fn fold_only(list: &KeyView, covered: usize, slice: usize, tree: &Tree) -> Self {
        RebuildJob {
            kind: JobKind::FoldOnly,
            stage: Stage::Fold,
            filter: list.filter.clone(),
            mode: list.mode,
            sort: list.sort,
            want_tree: true,
            covered,
            slice: slice.max(1),
            dirty: false,
            cursor: 0,
            order: Vec::new(),
            known: 0,
            scratch: Vec::new(),
            sort_phase: SortPhase::Chunks { next: 0 },
            inverse: Vec::new(),
            tree: Some(tree.shell()),
            fold: FoldProgress::default(),
        }
    }

    pub fn kind(&self) -> JobKind {
        self.kind
    }

    /// The tree mode the swap will leave on.
    pub fn want_tree(&self) -> bool {
        self.want_tree
    }

    /// How many loaded keys the finished list will cover.
    pub fn covered(&self) -> usize {
        self.covered
    }

    pub(super) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Whether scan pages arrived since the job started.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Stage-weighted progress, 0 to 99 (the swap is the 100).
    pub fn progress(&self, live: &KeyView) -> u8 {
        let n = match self.kind {
            JobKind::Full => self.order.len().max(1),
            JobKind::FoldOnly => live.len().max(1),
        };
        // Weights follow the measured cost shape: filter is cheap, sort and
        // fold are most of it.
        let has_sort = self.kind == JobKind::Full && self.sort != SortBy::Scan;
        let has_inverse = self.kind == JobKind::Full;
        let has_fold = self.want_tree;
        let w = |stage: Stage| -> f32 {
            match stage {
                Stage::Filter if self.kind == JobKind::Full => 1.0,
                Stage::Sort if has_sort => 4.0,
                Stage::Inverse if has_inverse => 1.0,
                Stage::Fold if has_fold => 4.0,
                _ => 0.0,
            }
        };
        let total: f32 = [Stage::Filter, Stage::Sort, Stage::Inverse, Stage::Fold]
            .into_iter()
            .map(w)
            .sum();
        let frac_in_stage = match self.stage {
            Stage::Filter => self.cursor as f32 / self.covered.max(1) as f32,
            Stage::Sort => self.sort_fraction(),
            Stage::Inverse => self.cursor as f32 / n as f32,
            Stage::Fold => self
                .tree
                .as_ref()
                .map_or(0.0, |t| t.fold_fraction(&self.fold, n)),
        };
        let mut done = 0.0;
        for s in [Stage::Filter, Stage::Sort, Stage::Inverse, Stage::Fold] {
            if s < self.stage {
                done += w(s);
            } else if s == self.stage {
                done += w(s) * frac_in_stage.clamp(0.0, 1.0);
            }
        }
        if total <= 0.0 {
            return 0;
        }
        ((done / total * 100.0) as u8).min(99)
    }

    fn sort_fraction(&self) -> f32 {
        let n = self.order.len().max(1);
        let chunks = n.div_ceil(self.slice);
        let passes = if chunks > 1 {
            chunks.next_power_of_two().trailing_zeros() as usize
        } else {
            0
        };
        let total = (1 + passes) as f32;
        match &self.sort_phase {
            SortPhase::Chunks { next } => (*next as f32 / n as f32) / total,
            SortPhase::Merge(m) => {
                let pass = (m.width / self.slice).max(1).trailing_zeros() as f32;
                (1.0 + pass + m.out as f32 / n as f32) / total
            }
        }
    }

    /// Advance by at most one slice. Returns whether the job is finished and
    /// ready to [`RebuildJob::finish`]. `live` is the shown view: a fold-only
    /// job reads its order in place.
    pub(super) fn step(&mut self, keys: &LoadedSet, live: &KeyView) -> bool {
        match self.stage {
            Stage::Filter => self.filter_step(keys),
            Stage::Sort => self.sort_step(keys),
            Stage::Inverse => return self.inverse_step(),
            Stage::Fold => return self.fold_step(keys, live),
        }
        false
    }

    fn filter_step(&mut self, keys: &LoadedSet) {
        let end = (self.cursor + self.slice).min(self.covered);
        let lazy = self.sort.is_lazy();
        for i in self.cursor..end {
            let keep = self.filter.is_empty()
                || keys
                    .name(i)
                    .is_some_and(|name| matches(name, &self.filter, self.mode));
            if keep {
                self.order.push(i as u32);
                if lazy && has_sort_value(keys, self.sort, i) {
                    self.known += 1;
                }
            }
        }
        self.cursor = end;
        if self.cursor >= self.covered {
            if !lazy {
                self.known = self.order.len();
            }
            self.cursor = 0;
            self.stage = if self.sort == SortBy::Scan || self.order.len() < 2 {
                Stage::Inverse
            } else {
                Stage::Sort
            };
        }
    }

    fn sort_step(&mut self, keys: &LoadedSet) {
        let n = self.order.len();
        let (slice, sort) = (self.slice, self.sort);
        match &mut self.sort_phase {
            SortPhase::Chunks { next } => {
                let lo = *next;
                let hi = (lo + slice).min(n);
                self.order[lo..hi].sort_by(|a, b| order_cmp(keys, sort, *a, *b));
                if hi >= n {
                    if n <= slice {
                        self.finish_sort();
                    } else {
                        self.scratch = vec![0; n];
                        self.sort_phase = SortPhase::Merge(Merge::pass(slice, n));
                    }
                } else {
                    *next = hi;
                }
            }
            SortPhase::Merge(m) => {
                let mut produced = 0;
                while produced < slice && m.out < n {
                    let (a, b) = (self.order[m.i.min(n - 1)], self.order[m.j.min(n - 1)]);
                    // Take the right run only when it is strictly smaller:
                    // ties keep the left, which keeps the merge stable.
                    let take_left = m.i < m.mid
                        && (m.j >= m.hi || order_cmp(keys, sort, b, a) != Ordering::Less);
                    if take_left {
                        self.scratch[m.out] = self.order[m.i];
                        m.i += 1;
                    } else {
                        self.scratch[m.out] = self.order[m.j];
                        m.j += 1;
                    }
                    m.out += 1;
                    produced += 1;
                    if m.out == m.hi {
                        let lo = m.hi;
                        m.mid = (lo + m.width).min(n);
                        m.hi = (lo + 2 * m.width).min(n);
                        m.i = lo;
                        m.j = m.mid;
                    }
                }
                if m.out >= n {
                    std::mem::swap(&mut self.order, &mut self.scratch);
                    let width = m.width * 2;
                    if width >= n {
                        self.finish_sort();
                    } else {
                        self.sort_phase = SortPhase::Merge(Merge::pass(width, n));
                    }
                }
            }
        }
    }

    fn finish_sort(&mut self) {
        self.scratch = Vec::new();
        self.cursor = 0;
        self.stage = Stage::Inverse;
    }

    fn inverse_step(&mut self) -> bool {
        if self.cursor == 0 {
            self.inverse = vec![NO_ROW; self.covered];
        }
        let end = (self.cursor + self.slice).min(self.order.len());
        for row in self.cursor..end {
            if let Some(slot) = self.inverse.get_mut(self.order[row] as usize) {
                *slot = row as u32;
            }
        }
        self.cursor = end;
        if self.cursor >= self.order.len() {
            self.cursor = 0;
            // The inverse is the last flat stage; `want_tree` folds next.
            self.stage = Stage::Fold;
            return !self.want_tree;
        }
        false
    }

    fn fold_step(&mut self, keys: &LoadedSet, live: &KeyView) -> bool {
        if !self.want_tree {
            return true;
        }
        let Some(tree) = self.tree.as_mut() else {
            return true;
        };
        let order: &[u32] = match self.kind {
            JobKind::Full => &self.order,
            JobKind::FoldOnly => live.order(),
        };
        if self.cursor == 0 {
            // First fold step: clear the shell and size its rows.
            self.fold = tree.fold_begin(order.len());
            self.cursor = 1;
        }
        tree.fold_step(keys, order, self.covered, &mut self.fold, self.slice)
    }

    /// The finished pieces, to be placed by the swap.
    pub(super) fn finish(self) -> Finished {
        Finished {
            list: (self.kind == JobKind::Full).then_some(FinishedList {
                order: self.order,
                inverse: self.inverse,
                known: self.known,
                built: (self.filter, self.mode, self.sort),
            }),
            tree: self.tree.filter(|_| self.want_tree),
            want_tree: self.want_tree,
            covered: self.covered,
        }
    }
}

pub(super) struct FinishedList {
    order: Vec<u32>,
    inverse: Vec<u32>,
    known: usize,
    built: (String, FilterMode, SortBy),
}

pub(super) struct Finished {
    list: Option<FinishedList>,
    tree: Option<Tree>,
    want_tree: bool,
    covered: usize,
}

impl State {
    /// Keys per rebuild step: [`REBUILD_SLICE`] unless a test or the
    /// calibration run overrode it.
    pub fn rebuild_slice(&self) -> usize {
        self.rebuild_slice.unwrap_or(REBUILD_SLICE).max(1)
    }

    /// Whether a rebuild job is running.
    pub fn rebuild_running(&self) -> bool {
        self.job.is_some()
    }

    /// `rebuilding N%`'s N, while a job runs.
    pub fn rebuild_progress(&self) -> Option<u8> {
        self.job.as_ref().map(|j| j.progress(&self.list))
    }

    /// The tree mode the list is heading for: the running job's target, else
    /// the shown mode. `State::tree_mode` itself stays the *shown* mode until
    /// the swap, because `State::tree` is only built for that.
    pub fn target_tree_mode(&self) -> bool {
        self.job.as_ref().map_or(self.tree_mode, |j| j.want_tree)
    }

    /// Flat, in scan order: the one view a scan page extends in place.
    pub(crate) fn list_is_scan_flat(&self) -> bool {
        !self.tree_mode && self.list.sort == SortBy::Scan
    }

    /// Whether `n` keys is more than one slice of work.
    fn is_large(&self, n: usize) -> bool {
        n >= self.rebuild_slice()
    }

    /// Mark the running job dirty (scan pages arrived). A no-op without one.
    pub(crate) fn mark_rebuild_dirty(&mut self) {
        if let Some(job) = &mut self.job {
            job.mark_dirty();
        }
    }

    /// [`State::rebuild_list`] as a user trigger: below one slice of keys it
    /// is the synchronous rebuild; otherwise a [`RebuildJob`] starts, replacing
    /// any running one, and the shown list stays as it is until it swaps.
    pub fn rebuild_list_async(&mut self) {
        if !self.is_large(self.keys.len()) {
            self.rebuild_list();
            return;
        }
        self.start_full_job(self.target_tree_mode());
    }

    /// A scan page's scheduled rebuild: never replaces a running job (a job
    /// outlasting the gap between pages would be restarted forever), only
    /// marks it dirty.
    pub fn rebuild_list_coalescing(&mut self) {
        if self.job.is_some() {
            self.mark_rebuild_dirty();
        } else {
            self.rebuild_list_async();
        }
    }

    /// [`State::refold`] as a user trigger: a fold-only job when the view is
    /// current and large, the synchronous refold when small, a full rebuild
    /// when the view is not current.
    pub fn refold_async(&mut self) {
        if !self.fold_is_current() {
            self.rebuild_list_async();
        } else if self.is_large(self.list.len()) {
            self.start_fold_job();
        } else {
            self.refold();
        }
    }

    /// Switch tree mode on or off as a user trigger (`t`).
    pub fn set_tree_mode_async(&mut self, want: bool) {
        self.job = None;
        if want {
            if self.view_is_current() {
                if self.is_large(self.list.len()) {
                    self.start_fold_job();
                } else {
                    self.tree_mode = true;
                    self.refold();
                }
            } else if self.is_large(self.keys.len()) {
                self.start_full_job(true);
            } else {
                self.tree_mode = true;
                self.rebuild_list();
            }
        } else if self.tree_mode && self.view_is_current() {
            self.tree_mode = false;
            self.leave_tree_mode();
        } else if self.is_large(self.keys.len()) {
            self.start_full_job(false);
        } else {
            self.tree_mode = false;
            self.rebuild_list();
        }
    }

    fn start_full_job(&mut self, want_tree: bool) {
        self.job = None;
        self.filter_pending = false;
        // Tree mode folds one pass over a name-ordered view: the same
        // invariant `rebuild_list_with` enforces, applied to the target.
        if want_tree && self.list.sort != SortBy::Name {
            self.list.sort = SortBy::Name;
        }
        self.job = Some(RebuildJob::full(
            self.list.filter.clone(),
            self.list.mode,
            self.list.sort,
            want_tree,
            self.keys.len(),
            self.rebuild_slice(),
            &self.tree,
        ));
    }

    fn start_fold_job(&mut self) {
        self.job = Some(RebuildJob::fold_only(
            &self.list,
            self.list.covered_len(),
            self.rebuild_slice(),
            &self.tree,
        ));
    }

    /// Advance the running job by one slice, swapping its result in when it
    /// finishes. Returns the key count the swapped list covers, if it swapped.
    pub fn rebuild_step(&mut self) -> Option<usize> {
        let mut job = self.job.take()?;
        if !job.step(&self.keys, &self.list) {
            self.job = Some(job);
            return None;
        }
        Some(self.swap_job(job))
    }

    /// Run the running job to completion, one step at a time: what the shell
    /// does across frames. For tests and the perf suite.
    pub fn run_rebuild_to_completion(&mut self) {
        while self.job.is_some() {
            self.rebuild_step();
        }
    }

    /// Replace the shown list with the finished job's, in one go, and put the
    /// selection and Open key back (`relocate_after_rows_changed`).
    fn swap_job(&mut self, job: RebuildJob) -> usize {
        // Captured now, on the shown structures: a move made while the job
        // ran is respected by key index, exactly as `rebuild_list` captures it
        // before the rows change.
        //
        // When the swap flips tree mode with the cursor still on row 0 (what
        // `t` leaves it at), the synchronous toggle captures it *after*
        // flipping the mode, through whichever structure that mode reads. Do
        // the same, so `t` lands the cursor exactly where it always did. A
        // cursor moved off row 0 during the job is a move to respect, so it is
        // read on the shown structures.
        if job.want_tree() != self.tree_mode && self.view.selected == 0 {
            self.tree_mode = job.want_tree();
        }
        let selected_index = self.key_at(self.view.selected);
        let was_dirty = job.is_dirty();
        let done = job.finish();
        let covered = done.covered;
        if let Some(list) = done.list {
            self.list
                .install(list.order, list.inverse, list.known, list.built, covered);
            self.scan_last_rebuild_len = covered;
        }
        if let Some(tree) = done.tree {
            self.tree = tree;
        }
        self.tree_mode = done.want_tree;
        self.relocate_after_rows_changed(selected_index);
        // Pages that arrived during the job: a flat scan-order view is
        // brought level by appending (`O(new keys)`); anything else is left
        // to the caller, which decides on the follow-up job.
        if was_dirty
            && self.keys.len() > covered
            && !self.tree_mode
            && self.list.sort == SortBy::Scan
            && self.list.covered_len() == covered
        {
            self.list.extend(&self.keys, covered);
            self.relocate_open_key();
        }
        covered
    }
}

/// Equivalence of a rebuild job run to completion, step by step, with
/// `rebuild_list` (M6 task 3): over random keyspaces in random order, filters,
/// every sort with partly-known metadata, tree mode on and off, random
/// collapsed sets, selections and Open keys. Deterministic PRNG, as in M4's
/// and task 2's tests. The slice is tiny so jobs span many steps.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::key::KeyName;
    use crate::state::OpenKey;
    use crate::state::loaded::{KeyKind, TTL_NONE};

    pub(crate) struct Rng(pub u64);

    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        pub fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    const SEGMENTS: [&str; 6] = ["a", "b", "ab", "c", "user", "x1"];
    pub(crate) const FILTERS: [(&str, FilterMode); 9] = [
        ("", FilterMode::Glob),
        ("a", FilterMode::Glob),
        ("a*", FilterMode::Glob),
        ("*b", FilterMode::Glob),
        ("a?b", FilterMode::Glob),
        ("user*:c", FilterMode::Glob),
        ("ac", FilterMode::Fuzzy),
        ("ub", FilterMode::Fuzzy),
        ("", FilterMode::Fuzzy),
    ];
    const SORTS: [SortBy; 5] = [
        SortBy::Scan,
        SortBy::Name,
        SortBy::Ttl,
        SortBy::Size,
        SortBy::Kind,
    ];

    pub(crate) fn random_name(rng: &mut Rng) -> String {
        let depth = 1 + rng.below(4);
        let mut parts: Vec<String> = (0..depth)
            .map(|_| SEGMENTS[rng.below(SEGMENTS.len())].to_string())
            .collect();
        // Few leaves, so duplicate names occur and ties are exercised.
        parts.push(format!("k{}", rng.below(40)));
        parts.join(":")
    }

    /// Fill in the lazy columns of a random subset of keys.
    fn random_metadata(rng: &mut Rng, keys: &mut LoadedSet) {
        for i in 0..keys.len() {
            if rng.below(3) == 0 {
                keys.set_kind(
                    i,
                    [KeyKind::String, KeyKind::Hash, KeyKind::List][rng.below(3)],
                );
            }
            if rng.below(3) == 0 {
                let ttl = if rng.below(3) == 0 {
                    TTL_NONE
                } else {
                    rng.below(500) as i32
                };
                keys.set_ttl(i, ttl, 0);
            }
            if rng.below(3) == 0 {
                keys.set_size(i, rng.below(1000) as u32);
            }
        }
    }

    pub(crate) fn random_prefix(rng: &mut Rng, state: &State) -> Option<String> {
        if state.keys.is_empty() {
            return None;
        }
        let name =
            String::from_utf8_lossy(state.keys.name(rng.below(state.keys.len()))?).into_owned();
        let cuts: Vec<usize> = name.match_indices(':').map(|(i, _)| i + 1).collect();
        let cut = *cuts.get(rng.below(cuts.len().max(1)))?;
        Some(name[..cut].to_string())
    }

    /// A built, current state: random keys in random order and metadata, a
    /// filter, a sort (Name in tree mode), a collapsed set, a selection and
    /// (usually) an Open key. `slice` is small enough to span several steps.
    pub(crate) fn random_state(rng: &mut Rng, tree: bool) -> State {
        let mut keys = LoadedSet::default();
        for _ in 0..(20 + rng.below(300)) {
            keys.push(random_name(rng).as_bytes());
        }
        random_metadata(rng, &mut keys);
        let (filter, mode) = FILTERS[rng.below(FILTERS.len())];
        let mut state = State {
            keys,
            rows: 30,
            tree_mode: tree,
            rebuild_slice: Some(1 + rng.below(70)),
            ..State::default()
        };
        state.list.filter = filter.to_string();
        state.list.mode = mode;
        state.list.sort = SORTS[rng.below(SORTS.len())];
        state.rebuild_list();
        if tree {
            for _ in 0..rng.below(6) {
                if let Some(prefix) = random_prefix(rng, &state) {
                    state.tree.toggle(&prefix);
                }
            }
            state.rebuild_list();
        }
        if state.row_count() > 0 {
            state.view.selected = rng.below(state.row_count());
        }
        if rng.below(4) != 0 && !state.keys.is_empty() {
            let index = rng.below(state.keys.len());
            let name = KeyName::from(state.keys.name(index).unwrap());
            state.open = Some(OpenKey::gone(Some(index), name, 0));
            state.relocate_open_key();
        }
        state
    }

    /// Everything a rebuild decides.
    pub(crate) fn assert_same(got: &State, want: &State, what: &str) {
        assert_eq!(got.tree, want.tree, "{what}: tree");
        assert_eq!(got.list, want.list, "{what}: list");
        assert_eq!(got.tree_mode, want.tree_mode, "{what}: tree_mode");
        assert_eq!(
            got.filter_pending, want.filter_pending,
            "{what}: filter_pending"
        );
        assert_eq!(got.view.selected, want.view.selected, "{what}: selected");
        assert_eq!(got.view.offset, want.view.offset, "{what}: offset");
        assert_eq!(
            got.open.as_ref().map(|o| o.row),
            want.open.as_ref().map(|o| o.row),
            "{what}: open row"
        );
        assert_eq!(
            got.scan_last_rebuild_len, want.scan_last_rebuild_len,
            "{what}: scan_last_rebuild_len"
        );
        assert!(got.job.is_none(), "{what}: job finished");
    }

    /// Step a started job to the end, asserting that the shown list is
    /// untouched until the one step that swaps.
    pub(crate) fn run_checking_shown_is_stable(state: &mut State, what: &str) -> usize {
        let snap = (state.list.clone(), state.tree.clone(), state.tree_mode);
        let mut steps = 0;
        while state.job.is_some() {
            let swapped = state.rebuild_step().is_some();
            steps += 1;
            assert!(steps < 100_000, "{what}: job never finished");
            if !swapped {
                assert!(
                    state.list == snap.0 && state.tree == snap.1 && state.tree_mode == snap.2,
                    "{what}: the shown list changed before the swap"
                );
            }
        }
        steps
    }

    #[derive(Debug, Clone)]
    pub(crate) enum Trigger {
        Filter(&'static str, FilterMode),
        Sort(SortBy),
        TreeOn,
        TreeOff,
        Toggle(String),
        Redo,
    }

    pub(crate) fn pick(rng: &mut Rng, state: &State) -> Trigger {
        match rng.below(6) {
            0 => {
                let (f, m) = FILTERS[rng.below(FILTERS.len())];
                Trigger::Filter(f, m)
            }
            1 if !state.tree_mode => Trigger::Sort(SORTS[rng.below(SORTS.len())]),
            2 if !state.tree_mode => Trigger::TreeOn,
            3 if state.tree_mode => Trigger::TreeOff,
            4 if state.tree_mode => match random_prefix(rng, state) {
                Some(p) => Trigger::Toggle(p),
                None => Trigger::Redo,
            },
            _ => Trigger::Redo,
        }
    }

    /// Apply a trigger the way the handlers do, with `job` choosing the
    /// sliced paths and otherwise the synchronous reference.
    pub(crate) fn apply(state: &mut State, t: &Trigger, job: bool) {
        match t {
            Trigger::Filter(f, m) => {
                state.list.filter = f.to_string();
                state.list.mode = *m;
                if job {
                    state.rebuild_list_async();
                } else {
                    state.rebuild_list();
                }
            }
            Trigger::Sort(s) => {
                state.list.sort = *s;
                if job {
                    state.rebuild_list_async();
                } else {
                    state.rebuild_list();
                }
            }
            Trigger::TreeOn | Trigger::TreeOff => {
                let want = matches!(t, Trigger::TreeOn);
                state.view.selected = 0;
                state.view.offset = 0;
                if job {
                    state.set_tree_mode_async(want);
                } else {
                    state.tree_mode = want;
                    state.rebuild_list();
                }
            }
            Trigger::Toggle(prefix) => {
                state.tree.toggle(prefix);
                if job {
                    state.refold_async();
                } else {
                    state.rebuild_list();
                }
            }
            Trigger::Redo => {
                if job {
                    state.rebuild_list_async();
                } else {
                    state.rebuild_list();
                }
            }
        }
    }

    #[test]
    fn a_job_run_to_completion_equals_rebuild_list() {
        let mut rng = Rng(101);
        let (mut jobs, mut multi_step, mut fold_only) = (0, 0, 0);
        for case in 0..1200 {
            let tree = rng.below(2) == 0;
            let mut state = random_state(&mut rng, tree);
            for round in 0..3 {
                let trigger = pick(&mut rng, &state);
                let mut want = state.clone();
                apply(&mut want, &trigger, false);
                let what = format!("case {case}.{round} {trigger:?}");
                apply(&mut state, &trigger, true);
                if let Some(job) = &state.job {
                    jobs += 1;
                    fold_only += usize::from(job.kind() == JobKind::FoldOnly);
                    let steps = run_checking_shown_is_stable(&mut state, &what);
                    multi_step += usize::from(steps > 2);
                }
                assert_same(&state, &want, &what);
                if state.row_count() > 0 {
                    state.view.selected = rng.below(state.row_count());
                    state.relocate_open_key();
                }
            }
        }
        assert!(jobs > 1500, "only {jobs} jobs ran");
        assert!(multi_step > 1000, "only {multi_step} multi-step jobs");
        assert!(fold_only > 100, "only {fold_only} fold-only jobs");
    }

    #[test]
    fn a_selection_moved_while_the_job_ran_is_respected_by_key() {
        let mut rng = Rng(103);
        for case in 0..300 {
            let mut state = random_state(&mut rng, false);
            state.list.sort = SortBy::Name;
            state.rebuild_list();
            state.list.filter = String::new();
            state.rebuild_list_async();
            if state.job.is_none() || state.row_count() == 0 {
                continue;
            }
            // Move the cursor on the old list mid-job.
            state.rebuild_step();
            state.view.selected = rng.below(state.row_count());
            let wanted = state.key_at(state.view.selected);
            state.run_rebuild_to_completion();
            if let Some(index) = wanted
                && let Some(row) = state.row_of(index)
            {
                assert_eq!(state.view.selected, row, "case {case}");
            }
        }
    }

    #[test]
    fn progress_climbs_and_stays_below_a_hundred() {
        let mut rng = Rng(105);
        for _ in 0..200 {
            let mut state = random_state(&mut rng, true);
            state.list.filter = String::new();
            state.rebuild_list_async();
            let mut last = 0;
            while state.job.is_some() {
                let p = state.rebuild_progress().unwrap();
                assert!(p < 100 && p + 3 >= last, "progress {p} after {last}");
                last = last.max(p);
                state.rebuild_step();
            }
        }
    }
}
