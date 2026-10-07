//! Release-mode performance budgets (M4 task 2, `docs/plans/m4-perf-harness.md`;
//! made honest by M6 task 1, `docs/plans/m6-harness.md`).
//!
//! Every test here is `#[ignore]`d: `cargo test --workspace` stays debug-mode
//! and Docker-free. These run via a dedicated CI job, in release, the same
//! shape as the Docker-backed integration suite already uses:
//!
//! ```text
//! cargo test -p redis-pane-core --release --test perf -- --ignored --nocapture --test-threads=1
//! ```
//!
//! One test at a time: these are wall-clock timings, and in parallel they
//! contend for CPU and read slower than they are.
//!
//! # Fixtures (M6 task 1)
//!
//! M4's harness pushed `user:{i:08}:session` in index order, which is already
//! name order. Real `SCAN` returns keys in hash-slot order, effectively
//! random, and a pass in name order over an arena in arrival order is a
//! cache miss per key; the old harness understated every rebuild by 5-10x
//! (`docs/plans/m6-perf-rebuild.md`). Three deterministic fixtures, all 1M
//! keys, shuffled with a fixed-seed splitmix64 written below (no crate dep):
//!
//! | Fixture       | Names                                       | Push order |
//! |---------------|---------------------------------------------|------------|
//! | `sorted`      | `user:{i:08}:session`                       | index      |
//! | `random_flat` | the same                                    | shuffled   |
//! | `random_deep` | `app:{i%7}:tenant:{i%211}:user:{i:08}:session` | shuffled |
//!
//! `sorted` is the control: its tests keep their M4 names and ceilings so the
//! old numbers stay comparable. `random_deep` is the reference fixture for
//! M6's goals; every pre-existing measurement runs on it, plus collapse,
//! expand, the fold alone, the Name sort alone and the whole-scan in tree
//! mode. `random_flat` is run for the rebuild-heavy tests only. Each fixture
//! is built once per process and cloned per test.
//!
//! # Budgets
//!
//! **The override that shapes every budget below:** a budget the current code
//! does not yet meet must not turn CI red. Where the current code already
//! meets the PRD §7 target (a keystroke answered in 16ms), the constant *is*
//! the target, labelled `AT TARGET`. Where it does not, the constant is a
//! **regression ceiling**, labelled `CEILING`, with the PRD target named
//! beside it. Every test prints its measured number first, so a CI log
//! carries the real value even though the assertion is against the (looser)
//! ceiling.
//!
//! Ceiling rule: M4 used **1.5x the CI measurement, rounded up**. On the M6
//! fixtures three runs of the same code on CI differed by up to 1.9x (runner
//! hardware varies), so every ceiling is **1.5x the slowest figure observed
//! across the local runs and PR #78's three CI runs, rounded up**; the table
//! is in `docs/plans/m6-harness.md`. Time-to-new-list tests
//! (`time_to_new_list_*`) measure the time from the trigger until the rebuild
//! job's swap (M6 task 3), every update summed.
//!
//! **Since M6 task 3** every trigger that rebuilds a large list is a sliced job,
//! and the figure these tests assert is the **worst single `update`**: the
//! trigger and each `Msg::RebuildStep`, timed on its own (`Profile`). That is
//! the claim the milestone makes, "no frame waits on a rebuild", and it is
//! held at 16ms (AT TARGET): the slowest step was about 8-9ms on a laptop and on
//! CI after task 3 and is 1-3ms since task 5's fast fold, so the 1.5x rule lands
//! well under the target. The worst update across the
//! runs is printed beside the median so a noisy runner is visible in the log.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use redis_pane_core::clock::FixedClock;
use redis_pane_core::msg::{KeyCode, KeyPress};
use redis_pane_core::render;
use redis_pane_core::state::{
    Connection, Environment, FilterMode, KeyView, Link, LoadedSet, SortBy, Source, State, Tracking,
    Tree,
};
use redis_pane_core::theme::{ColorDepth, Theme};
use redis_pane_core::{Msg, update};

const COLS: u16 = 130;
const ROWS: u16 = 26;
const N: usize = 1_000_000;
/// Fixed shuffle seed: every run, every machine, same arrival order.
const SEED: u64 = 0x5eed_5ca9_1234_abcd;

// ── deterministic PRNG and fixtures ───────────────────────────────────────

/// splitmix64: tiny, fixed-seed, good enough to shuffle.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Fisher-Yates permutation of `0..n`: arrival position -> key index.
fn permutation(n: usize) -> Vec<u32> {
    let mut v: Vec<u32> = (0..n as u32).collect();
    let mut rng = SplitMix64(SEED);
    for i in (1..n).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fixture {
    Sorted,
    RandomFlat,
    RandomDeep,
}

fn order() -> &'static [u32] {
    static ORDER: OnceLock<Vec<u32>> = OnceLock::new();
    ORDER.get_or_init(|| permutation(N))
}

impl Fixture {
    fn label(self) -> &'static str {
        match self {
            Fixture::Sorted => "sorted",
            Fixture::RandomFlat => "random_flat",
            Fixture::RandomDeep => "random_deep",
        }
    }

    /// Name of the key with index `i`.
    fn name(self, i: usize) -> String {
        match self {
            Fixture::Sorted | Fixture::RandomFlat => format!("user:{i:08}:session"),
            Fixture::RandomDeep => {
                format!("app:{}:tenant:{}:user:{i:08}:session", i % 7, i % 211)
            }
        }
    }

    /// Name of the key that arrives at scan position `pos` (a position past
    /// the first million is simply the next index).
    fn scan_name(self, pos: usize) -> String {
        match self {
            Fixture::Sorted => self.name(pos),
            _ if pos < N => self.name(order()[pos] as usize),
            _ => self.name(pos),
        }
    }

    /// The 1M-key Loaded set, built once per fixture per process.
    fn keys(self) -> LoadedSet {
        static SORTED: OnceLock<LoadedSet> = OnceLock::new();
        static FLAT: OnceLock<LoadedSet> = OnceLock::new();
        static DEEP: OnceLock<LoadedSet> = OnceLock::new();
        let cell = match self {
            Fixture::Sorted => &SORTED,
            Fixture::RandomFlat => &FLAT,
            Fixture::RandomDeep => &DEEP,
        };
        cell.get_or_init(|| self.build_keys()).clone()
    }

    /// Push the fixture's keys one by one, as a scan does. `keys()` clones a
    /// cached copy, which is exact-fit; `heap_bytes` counts capacity, so the
    /// memory test builds fresh to report what a real scan holds.
    fn build_keys(self) -> LoadedSet {
        let mut keys = LoadedSet::default();
        for pos in 0..N {
            assert!(keys.push(self.scan_name(pos).as_bytes()));
        }
        keys
    }

    /// A page of `count` keys starting at scan position `from`.
    fn page(self, from: usize, count: usize) -> Vec<Vec<u8>> {
        (from..from + count)
            .map(|p| self.scan_name(p).into_bytes())
            .collect()
    }
}

fn base_state() -> State {
    State {
        cols: COLS,
        rows: ROWS,
        connection: Connection {
            target: "perf-harness:6379/0".into(),
            environment: Environment::Staging,
            source: Source::Profile("perf".into()),
            topology: None,
        },
        last_read_ms: Some(0),
        link: Link::Up {
            version: "8.4.0".into(),
            tracking: Tracking::Armed,
        },
        ..State::default()
    }
}

/// A deterministic, columnar `State` holding the fixture's 1M keys with the
/// list rebuilt, in the given view mode — the steady state a browse sits in
/// once a scan has finished, which is what every per-keystroke test below
/// perturbs with one message. `tree_mode` and `sort` are independent knobs:
/// tree mode forces a Name sort (`rebuild_list`'s own invariant) regardless
/// of what `sort` asks for.
fn make_state(fx: Fixture, tree_mode: bool, sort: SortBy) -> State {
    let mut state = State {
        keys: fx.keys(),
        list: KeyView::new("", FilterMode::Glob, sort),
        tree_mode,
        ..base_state()
    };
    state.rebuild_list();
    state
}

/// Flat view, scan order — the shape most pre-existing tests use.
fn big_state(fx: Fixture) -> State {
    make_state(fx, false, SortBy::Scan)
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn render_once(state: &State) {
    let _ = render::frame(
        state,
        &Theme::new(ColorDepth::Monochrome),
        &FixedClock(0),
        Rect::new(0, 0, COLS, ROWS),
    );
}

/// Median wall time of `update(state.clone(), msg)` followed by a render of
/// the resulting frame — a keystroke's full cost, not just the state change.
/// Cloning the base state happens *outside* the timed section: `update`
/// takes `State` by value, but what this measures is the update/render cost
/// a keystroke pays, not the cost of duplicating a 1M-key arena to set the
/// scene up again for the next sample.
fn time_update_and_render(base: &State, iterations: usize, msg: impl Fn() -> Msg) -> Duration {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let working = base.clone();
        let m = msg();
        let started = Instant::now();
        let (state, _cmds) = update(working, m);
        render_once(&state);
        samples.push(started.elapsed());
    }
    median(samples)
}

/// Median wall time of the `update` call alone. Superseded by
/// [`time_to_swap`] for anything that starts a rebuild job; kept for the
/// triggers that do not.
#[allow(dead_code)]
fn time_to_new_list(base: &State, iterations: usize, msg: impl Fn() -> Msg) -> Duration {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let working = base.clone();
        let m = msg();
        let started = Instant::now();
        let (state, _cmds) = update(working, m);
        samples.push(started.elapsed());
        drop(state);
    }
    median(samples)
}

/// What one trigger costs once a large rebuild is a sliced job (M6 task 3):
/// the update that starts it, then every `Msg::RebuildStep` until the swap, each
/// timed on its own, because "no frame waits" is a claim about the worst single
/// `update`, not about the total.
#[derive(Clone)]
struct Profile {
    /// The trigger's own `update`, plus the frame render a keystroke pays.
    trigger: Duration,
    /// Each step, with the stage it ran.
    steps: Vec<(&'static str, Duration)>,
    /// Trigger to swap: every update summed, which is what the reader waits
    /// for before the new list is on screen (excludes drawing between steps).
    to_swap: Duration,
}

impl Profile {
    fn worst(&self) -> Duration {
        self.steps
            .iter()
            .map(|(_, d)| *d)
            .chain(std::iter::once(self.trigger))
            .max()
            .unwrap_or_default()
    }

    /// The slowest step of each stage that ran, in stage order.
    fn worst_per_stage(&self) -> Vec<(&'static str, Duration)> {
        let mut out: Vec<(&'static str, Duration)> = Vec::new();
        for (name, d) in &self.steps {
            match out.iter_mut().find(|(n, _)| n == name) {
                Some((_, w)) => *w = (*w).max(*d),
                None => out.push((name, *d)),
            }
        }
        out
    }
}

impl Profile {
    /// The summed time of each stage that ran, in stage order: which stage
    /// is the longest, which is what decides whether a stage is worth a
    /// faster path (M6 task 6's gate).
    fn stage_totals(&self) -> Vec<(&'static str, Duration)> {
        let mut out: Vec<(&'static str, Duration)> = Vec::new();
        for (name, d) in &self.steps {
            match out.iter_mut().find(|(n, _)| n == name) {
                Some((_, t)) => *t += *d,
                None => out.push((name, *d)),
            }
        }
        out
    }
}

/// Run `msg` against a clone of `base` and then step any job it started to the
/// swap, timing every update.
fn profile_once(base: &State, msg: Msg) -> Profile {
    let working = base.clone();
    let started = Instant::now();
    let (mut state, _cmds) = update(working, msg);
    render_once(&state);
    let trigger = started.elapsed();
    let mut steps = Vec::new();
    let mut to_swap = trigger;
    while let Some(stage) = state.job.as_ref().map(|j| j.stage_name()) {
        let t = Instant::now();
        let (next, _cmds) = update(state, Msg::RebuildStep);
        let d = t.elapsed();
        state = next;
        steps.push((stage, d));
        to_swap += d;
    }
    Profile {
        trigger,
        steps,
        to_swap,
    }
}

/// `profile_once` `n` times; the run whose worst update is the median.
fn profile(base: &State, n: usize, msg: impl Fn() -> Msg) -> Profile {
    let mut runs: Vec<Profile> = (0..n).map(|_| profile_once(base, msg())).collect();
    runs.sort_by_key(Profile::worst);
    let worst_ever = runs.last().map(Profile::worst).unwrap_or_default();
    let mut median = runs[runs.len() / 2].clone();
    // The median run is the figure; the worst update across every run is
    // printed beside it so a noisy runner is visible in the log.
    median.steps.shrink_to_fit();
    println!(
        "    worst single update, median run {:?} (worst of {n} runs {worst_ever:?}); {} steps, trigger {:?}, trigger to swap {:?}; worst per stage {:?}",
        median.worst(),
        median.steps.len(),
        median.trigger,
        median.to_swap,
        median.worst_per_stage(),
    );
    println!("    stage totals {:?}", median.stage_totals());
    median
}

/// Median trigger-to-swap time over `n` runs.
fn time_to_swap(base: &State, n: usize, msg: impl Fn() -> Msg) -> Duration {
    median(
        (0..n)
            .map(|_| profile_once(base, msg()).to_swap)
            .collect::<Vec<_>>(),
    )
}

/// Samples for tests that run a full rebuild on a random fixture (~0.2-0.5s
/// each): fewer than the cheap tests' 11, still a median.
const SLOW_SAMPLES: usize = 5;

/// Wrapper generating one `#[test] #[ignore]` per (fixture, ceiling_ms).
macro_rules! variants {
    ($body:ident: $($name:ident => ($fx:expr, $ceil:expr)),+ $(,)?) => {
        $(
            #[test]
            #[ignore]
            fn $name() {
                $body($fx, $ceil)
            }
        )+
    };
}

fn assert_budget(what: &str, elapsed: Duration, ceil_ms: u64) {
    if ceil_ms <= 16 {
        // AT TARGET: PRD §7's 16ms, one frame.
        assert!(
            elapsed < Duration::from_millis(16),
            "{what} took {elapsed:?}, budget is 16ms"
        );
    } else {
        // CEILING: PRD §7 target is 16ms, not met.
        assert!(
            elapsed < Duration::from_millis(ceil_ms),
            "{what} took {elapsed:?}, ceiling is {ceil_ms}ms (target 16ms)"
        );
    }
}

fn key(code: KeyCode) -> Msg {
    Msg::Key(KeyPress::plain(code))
}

// Ceilings are 1.5x the slowest figure observed (local and three CI runs),
// rounded up; see the header.

// ── (a) scroll keystroke ─────────────────────────────────────────────────

fn scroll_keystroke(fx: Fixture, ceil_ms: u64) {
    let base = big_state(fx);
    let elapsed = time_update_and_render(&base, 11, || key(KeyCode::Down));
    println!("[{}] scroll keystroke @ 1M keys: {elapsed:?}", fx.label());
    // AT TARGET: PRD §7 — a keystroke is answerable in one frame (16ms).
    // Moving the selection touches no arena, no sort, no rebuild; only the
    // render cost scales with the keyspace, bounded by virtualization (R2.6).
    assert_budget("scroll", elapsed, ceil_ms);
}
variants!(scroll_keystroke:
    scroll_keystroke_at_1m_keys => (Fixture::Sorted, 16),
    scroll_keystroke_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

// ── (b) filter keystroke ─────────────────────────────────────────────────

fn filter_keystroke(fx: Fixture, ceil_ms: u64) {
    let mut base = big_state(fx);
    // Mid-typing: a filter is already active and capturing, and one more
    // character arrives. That extends the previous query, so the list
    // narrows from the rows already shown (M4 task 4, decision 1) instead of
    // re-matching the whole Loaded set.
    base.list.filter = "user:0000".into();
    base.filtering = true;
    base.rebuild_list();
    let elapsed = time_update_and_render(&base, 11, || key(KeyCode::Char('1')));
    println!("[{}] filter keystroke @ 1M keys: {elapsed:?}", fx.label());
    // AT TARGET: PRD §7's 16ms (narrowing from the previous result).
    assert_budget("filter keystroke", elapsed, ceil_ms);
}
variants!(filter_keystroke:
    filter_keystroke_at_1m_keys => (Fixture::Sorted, 16),
    filter_keystroke_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

/// A filter is active over the whole set and rows are in step with it.
fn filtered_state(fx: Fixture, filter: &str) -> State {
    let mut base = big_state(fx);
    base.list.filter = filter.into();
    base.filtering = true;
    base.rebuild_list();
    base
}

fn filter_first_character(fx: Fixture, ceil_ms: u64) {
    // The one narrowing keystroke that still touches the whole keyspace: the
    // previous result *is* the Loaded set; it matches every key once, no sort.
    let base = filtered_state(fx, "");
    let elapsed = time_update_and_render(&base, 11, || key(KeyCode::Char('u')));
    println!(
        "[{}] filter first character @ 1M keys: {elapsed:?}",
        fx.label()
    );
    // Sorted control: AT TARGET (~3.9ms). On random_deep it matches every
    // key through the shuffled arena and is a CEILING (~21ms).
    assert_budget("first filter character", elapsed, ceil_ms);
}
variants!(filter_first_character:
    filter_first_character_at_1m_keys => (Fixture::Sorted, 16),
    filter_first_character_at_1m_keys_random_deep => (Fixture::RandomDeep, 52));

fn filter_backspace_keystroke(fx: Fixture, ceil_ms: u64) {
    // M4 task 4 decision 2: a keystroke that cannot be narrowed defers its
    // rebuild to the shell's debounce timer, so what the reader waits on is
    // only the update that records the text plus the frame.
    let base = filtered_state(fx, "user:0000");
    let elapsed = time_update_and_render(&base, 11, || key(KeyCode::Backspace));
    println!(
        "[{}] filter backspace keystroke (rebuild deferred) @ 1M keys: {elapsed:?}",
        fx.label()
    );
    assert_budget("deferred keystroke", elapsed, ceil_ms);
}
variants!(filter_backspace_keystroke:
    filter_backspace_keystroke_at_1m_keys => (Fixture::Sorted, 16),
    filter_backspace_keystroke_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

/// The state `Msg::FilterRebuildDue` acts on: a backspace has deferred a rebuild.
fn debounce_pending_state(fx: Fixture) -> State {
    let (base, _) = update(filtered_state(fx, "user:0000"), key(KeyCode::Backspace));
    assert!(base.filter_pending);
    base
}

fn filter_rebuild_after_debounce(fx: Fixture, ceil_ms: u64) {
    // The deferred work itself: a full rebuild of the Loaded set, run once
    // per keystroke burst.
    let base = debounce_pending_state(fx);
    let n = if fx == Fixture::Sorted {
        11
    } else {
        SLOW_SAMPLES
    };
    println!("[{}] filter rebuild after debounce @ 1M keys", fx.label());
    let elapsed = profile(&base, n, || Msg::FilterRebuildDue).worst();
    // The worst single update of the sliced job (M6 task 3): the trigger and
    // every step. PRD target 16ms.
    assert_budget("post-debounce rebuild, worst update", elapsed, ceil_ms);
}
variants!(filter_rebuild_after_debounce:
    filter_rebuild_after_debounce_at_1m_keys => (Fixture::Sorted, 16),
    filter_rebuild_after_debounce_at_1m_keys_random_flat => (Fixture::RandomFlat, 16),
    filter_rebuild_after_debounce_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

fn time_to_new_list_filter_rebuild(fx: Fixture, ceil_ms: u64) {
    let base = debounce_pending_state(fx);
    let elapsed = time_to_swap(&base, SLOW_SAMPLES, || Msg::FilterRebuildDue);
    println!(
        "[{}] time-to-new-list (to the swap), debounced filter rebuild @ 1M keys: {elapsed:?}",
        fx.label()
    );
    // CEILING.
    assert_budget("time-to-new-list (filter rebuild)", elapsed, ceil_ms);
}
variants!(time_to_new_list_filter_rebuild:
    time_to_new_list_filter_rebuild_random_deep => (Fixture::RandomDeep, 88));

// ── (c) sort change ──────────────────────────────────────────────────────

fn sort_change(fx: Fixture, ceil_ms: u64) {
    let base = big_state(fx);
    let n = if fx == Fixture::Sorted {
        11
    } else {
        SLOW_SAMPLES
    };
    println!("[{}] sort change (scan -> name) @ 1M keys", fx.label());
    let elapsed = profile(&base, n, || key(KeyCode::Char('s'))).worst();
    // The worst single update of the sliced job (M6 task 3): the trigger and
    // every step.
    assert_budget("sort change, worst update", elapsed, ceil_ms);
}
variants!(sort_change:
    sort_change_at_1m_keys => (Fixture::Sorted, 16),
    sort_change_at_1m_keys_random_flat => (Fixture::RandomFlat, 16),
    sort_change_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

fn time_to_new_list_sort_change(fx: Fixture, ceil_ms: u64) {
    let base = big_state(fx);
    let elapsed = time_to_swap(&base, SLOW_SAMPLES, || key(KeyCode::Char('s')));
    println!(
        "[{}] time-to-new-list (to the swap), sort change @ 1M keys: {elapsed:?}",
        fx.label()
    );
    assert_budget("time-to-new-list (sort change)", elapsed, ceil_ms);
}
variants!(time_to_new_list_sort_change:
    time_to_new_list_sort_change_random_deep => (Fixture::RandomDeep, 231));

// ── (d) toggling tree mode ───────────────────────────────────────────────

fn toggle_tree(fx: Fixture, ceil_ms: u64) {
    let base = big_state(fx);
    let n = if fx == Fixture::Sorted {
        11
    } else {
        SLOW_SAMPLES
    };
    println!("[{}] toggle tree mode @ 1M keys", fx.label());
    // Entering tree mode forces a Name sort *and* a one-pass fold over all
    // 1M rows: a full job, filter, sort, inverse and fold stages. The figure
    // is the worst single update across them (M6 task 3).
    let elapsed = profile(&base, n, || key(KeyCode::Char('t'))).worst();
    assert_budget("tree toggle, worst update", elapsed, ceil_ms);
}
variants!(toggle_tree:
    toggle_tree_at_1m_keys => (Fixture::Sorted, 16),
    toggle_tree_at_1m_keys_random_flat => (Fixture::RandomFlat, 16),
    toggle_tree_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

/// `t` over a flat view that is already Name-sorted (M6 task 2): a fold, no
/// re-sort. `toggle_tree` above starts from scan order, which still sorts.
fn toggle_tree_from_name_sorted(fx: Fixture, ceil_ms: u64) {
    let base = make_state(fx, false, SortBy::Name);
    println!(
        "[{}] toggle tree mode, flat view already Name-sorted @ 1M keys",
        fx.label()
    );
    let elapsed = profile(&base, SLOW_SAMPLES, || key(KeyCode::Char('t'))).worst();
    assert_budget(
        "tree toggle (Name-sorted flat view), worst update",
        elapsed,
        ceil_ms,
    );
}
variants!(toggle_tree_from_name_sorted:
    toggle_tree_from_name_sorted_random_deep => (Fixture::RandomDeep, 16));

/// `t` out of tree mode over a current view: no fold, no sort.
fn toggle_tree_off(fx: Fixture, ceil_ms: u64) {
    let base = make_state(fx, true, SortBy::Name);
    let elapsed = time_update_and_render(&base, SLOW_SAMPLES, || key(KeyCode::Char('t')));
    println!(
        "[{}] toggle tree mode off @ 1M keys: {elapsed:?}",
        fx.label()
    );
    assert_budget("tree toggle off", elapsed, ceil_ms);
}
variants!(toggle_tree_off:
    toggle_tree_off_random_deep => (Fixture::RandomDeep, 10));

fn time_to_new_list_tree_toggle(fx: Fixture, ceil_ms: u64) {
    let base = big_state(fx);
    let elapsed = time_to_swap(&base, SLOW_SAMPLES, || key(KeyCode::Char('t')));
    println!(
        "[{}] time-to-new-list (to the swap), tree toggle @ 1M keys: {elapsed:?}",
        fx.label()
    );
    assert_budget("time-to-new-list (tree toggle)", elapsed, ceil_ms);
}
variants!(time_to_new_list_tree_toggle:
    // CEILING: 1.5x the slowest of three CI runs (194ms) = 292, M6 task 5. The
    // goal is <= 250ms; local runs are 108-115ms.
    time_to_new_list_tree_toggle_random_deep => (Fixture::RandomDeep, 292));

// ── (d2) collapse / expand one top-level group, fold alone, sort alone ──

/// Tree mode, row 0 a top-level expanded group, cursor on it.
fn tree_state(fx: Fixture) -> State {
    let state = make_state(fx, true, SortBy::Name);
    assert!(matches!(
        state.tree.row(0),
        Some(redis_pane_core::state::tree::Row::Group { expanded: true, .. })
    ));
    state
}

fn collapse_group(fx: Fixture, ceil_ms: u64) {
    // The real path: Left on an expanded group row -> `collapse_group` in
    // `update/keys.rs` -> `Tree::toggle` + `State::refold` (the fold alone,
    // M6 task 2).
    let base = tree_state(fx);
    let rows_before = base.tree.len();
    let (mut collapsed, _) = update(base.clone(), key(KeyCode::Left));
    assert!(collapsed.rebuild_running(), "a fold-only job at 1M keys");
    collapsed.run_rebuild_to_completion();
    assert!(
        collapsed.tree.len() < rows_before,
        "Left must collapse row 0"
    );
    drop(collapsed);
    println!("[{}] collapse one top-level group @ 1M keys", fx.label());
    // The fold alone since M6 task 2, sliced since task 3: the figure is the
    // worst single update.
    let elapsed = profile(&base, SLOW_SAMPLES, || key(KeyCode::Left)).worst();
    assert_budget("collapse, worst update", elapsed, ceil_ms);
}
variants!(collapse_group:
    collapse_group_at_1m_keys_random_flat => (Fixture::RandomFlat, 16),
    collapse_group_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

fn expand_group(fx: Fixture, ceil_ms: u64) {
    let (mut collapsed, _) = update(tree_state(fx), key(KeyCode::Left));
    collapsed.run_rebuild_to_completion();
    let rows_collapsed = collapsed.tree.len();
    let (mut expanded, _) = update(collapsed.clone(), key(KeyCode::Right));
    expanded.run_rebuild_to_completion();
    assert!(
        expanded.tree.len() > rows_collapsed,
        "Right on a collapsed group must expand it"
    );
    drop(expanded);
    println!("[{}] expand one top-level group @ 1M keys", fx.label());
    let elapsed = profile(&collapsed, SLOW_SAMPLES, || key(KeyCode::Right)).worst();
    assert_budget("expand, worst update", elapsed, ceil_ms);
}
variants!(expand_group:
    expand_group_at_1m_keys_random_flat => (Fixture::RandomFlat, 16),
    expand_group_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

fn time_to_new_list_collapse(fx: Fixture, ceil_ms: u64) {
    let base = tree_state(fx);
    let elapsed = time_to_swap(&base, SLOW_SAMPLES, || key(KeyCode::Left));
    println!(
        "[{}] time-to-new-list (to the swap), collapse @ 1M keys: {elapsed:?}",
        fx.label()
    );
    assert_budget("time-to-new-list (collapse)", elapsed, ceil_ms);
}
variants!(time_to_new_list_collapse:
    // CEILING: 1.5x the slowest of three CI runs (36ms) = 55, M6 task 5.
    time_to_new_list_collapse_random_deep => (Fixture::RandomDeep, 55));

fn fold_alone(fx: Fixture, ceil_ms: u64) {
    // `Tree::rebuild` over a name-sorted KeyView, nothing else.
    let state = make_state(fx, false, SortBy::Name);
    let mut samples = Vec::with_capacity(SLOW_SAMPLES);
    let mut rows = 0;
    for _ in 0..SLOW_SAMPLES {
        let mut tree = Tree::default();
        let started = Instant::now();
        tree.rebuild(&state.keys, &state.list);
        samples.push(started.elapsed());
        rows = tree.len();
    }
    let elapsed = median(samples);
    println!(
        "[{}] fold alone (Tree::rebuild over a name-sorted view) @ 1M keys: {elapsed:?} ({rows} rows)",
        fx.label()
    );
    // M6 task 5 target: <= 60ms local (met: 27-33ms; CI 37-48ms). CEILING: 1.5x the
    // slowest of three CI runs (sorted 26ms, random_flat 48ms, random_deep 45ms).
    assert_budget("fold alone", elapsed, ceil_ms);
}
variants!(fold_alone:
    fold_alone_at_1m_keys => (Fixture::Sorted, 40),
    fold_alone_at_1m_keys_random_flat => (Fixture::RandomFlat, 73),
    fold_alone_at_1m_keys_random_deep => (Fixture::RandomDeep, 68));

fn name_sort_alone(fx: Fixture, ceil_ms: u64) {
    // `KeyView::rebuild` with SortBy::Name and no filter, nothing else.
    let keys = fx.keys();
    let mut samples = Vec::with_capacity(SLOW_SAMPLES);
    for _ in 0..SLOW_SAMPLES {
        let mut view = KeyView::new("", FilterMode::Glob, SortBy::Name);
        let started = Instant::now();
        view.rebuild(&keys);
        samples.push(started.elapsed());
    }
    let elapsed = median(samples);
    println!(
        "[{}] Name sort alone (KeyView::rebuild, no filter) @ 1M keys: {elapsed:?}",
        fx.label()
    );
    // M6 task 4 target: <= 80ms local on random_deep (71ms measured; CI 120ms).
    // All three are CEILINGS, 1.5x the slowest CI run: the record sort pays
    // its refinement rounds even on already-sorted names (13ms CI, was 4ms
    // when pdqsort saw them in order), and wins 2-3x on the random fixtures.
    assert_budget("name sort", elapsed, ceil_ms);
}
variants!(name_sort_alone:
    name_sort_alone_at_1m_keys => (Fixture::Sorted, 21),
    name_sort_alone_at_1m_keys_random_flat => (Fixture::RandomFlat, 94),
    name_sort_alone_at_1m_keys_random_deep => (Fixture::RandomDeep, 180));

// ── (e) one ScanBatch page arriving into an already-1M-key set ──────────

fn scan_batch_flat(fx: Fixture, ceil_ms: u64) {
    // Flat scan order: `KeyView::extend`'s `O(page)` path (M4 task 3).
    let base = big_state(fx);
    let page = fx.page(N, 500);
    let elapsed = time_update_and_render(&base, 11, || Msg::ScanBatch { keys: page.clone() });
    println!(
        "[{}] ScanBatch of 500 into 1M keys: {elapsed:?}",
        fx.label()
    );
    // AT TARGET: PLAN's "update + render at 1M keys within 16ms".
    assert_budget("ScanBatch(500)", elapsed, ceil_ms);
}
variants!(scan_batch_flat:
    scan_batch_of_500_into_1m_keys => (Fixture::Sorted, 16),
    scan_batch_of_500_into_1m_keys_random_deep => (Fixture::RandomDeep, 16));

fn scan_batch_tree(fx: Fixture, ceil_ms: u64) {
    // Tree mode is the app's *default* view (DESIGN §1). `make_state` has
    // rebuilt at 1,000,000 keys, so the one extra page is nowhere near the
    // next scheduled rebuild at 1,250,000 (M4 task 3's geometric schedule):
    // this is the "most pages are free" case. The worst page is measured by
    // `whole_scan_fold_in_tree_mode_from_empty`.
    let base = make_state(fx, true, SortBy::Scan);
    let page = fx.page(N, 500);
    let elapsed = time_update_and_render(&base, 11, || Msg::ScanBatch { keys: page.clone() });
    println!(
        "[{}] ScanBatch of 500 into 1M keys, tree mode: {elapsed:?}",
        fx.label()
    );
    // AT TARGET (tightened by M4 task 3 from a 195ms CEILING).
    assert_budget("ScanBatch(500) tree mode", elapsed, ceil_ms);
}
variants!(scan_batch_tree:
    scan_batch_of_500_into_1m_keys_tree_mode => (Fixture::Sorted, 16),
    scan_batch_of_500_into_1m_keys_tree_mode_random_deep => (Fixture::RandomDeep, 16));

fn scan_batch_flat_name_sorted(fx: Fixture, ceil_ms: u64) {
    // Flat, but sorted by Name: a page that does not land on a scheduled
    // rebuild is a cheap append; one that does pays a full re-sort.
    let base = make_state(fx, false, SortBy::Name);
    let page = fx.page(N, 500);
    let elapsed = time_update_and_render(&base, 11, || Msg::ScanBatch { keys: page.clone() });
    println!(
        "[{}] ScanBatch of 500 into 1M keys, flat sorted by name: {elapsed:?}",
        fx.label()
    );
    // AT TARGET: PRD §7's 16ms.
    assert_budget("ScanBatch(500) flat name-sorted", elapsed, ceil_ms);
}
variants!(scan_batch_flat_name_sorted:
    scan_batch_of_500_into_1m_keys_flat_sorted_by_name => (Fixture::Sorted, 16),
    scan_batch_of_500_into_1m_keys_flat_sorted_by_name_random_deep => (Fixture::RandomDeep, 16));

// ── whole scan, folded page by page, in tree mode (the default view) ────

/// Worst-page ceiling and total gate for the whole scan.
struct ScanBudget {
    worst_ms: u64,
    total_s: u64,
}

fn whole_scan_fold(fx: Fixture, budget: ScanBudget) {
    // Tree mode is the default view, so this is what a fresh launch against
    // a 1M-key server does: 2,000 pages of 500 keys, folded one at a time
    // via `update(state, Msg::ScanBatch { .. })`, as
    // `crates/app/src/redis/scan.rs`'s dispatch loop does. Pages arrive in
    // the fixture's scan order (random for the random fixtures: the real
    // case). M4 task 3's geometric rebuild schedule keeps most pages an
    // `O(page)` append; the ~30 pages that cross 25% growth pay a full
    // synchronous Name sort + `Tree::rebuild`, and late in the scan those
    // are the frame-freezing ones. Reports the worst page AND the total.
    //
    // Capped by wall time, not page count, so a regression to O(n²) times
    // out and reports how far it got rather than hanging the job.
    const TOTAL_PAGES: usize = 2_000; // 2,000 × 500 = 1,000,000 keys
    const WALL_CAP: Duration = Duration::from_secs(240);

    let mut state = State {
        list: KeyView::new("", FilterMode::Glob, SortBy::Scan),
        tree_mode: true,
        ..base_state()
    };

    let run_started = Instant::now();
    let mut page_times = Vec::with_capacity(2 * TOTAL_PAGES);
    let mut worst_page_time = Duration::ZERO;
    let mut worst_page_index = 0usize;
    let mut pages_done = 0usize;
    let mut steps_run = 0usize;
    for page in 0..TOTAL_PAGES {
        let batch = fx.page(page * 500, 500);
        let started = Instant::now();
        let (next, _cmds) = update(state, Msg::ScanBatch { keys: batch });
        let elapsed = started.elapsed();
        state = next;
        if elapsed > Duration::from_millis(16) {
            println!("  slow page update {page}: {elapsed:?}");
        }
        page_times.push(elapsed);
        if elapsed > worst_page_time {
            worst_page_time = elapsed;
            worst_page_index = page;
        }
        // The shell steps a running rebuild job between pages (input and
        // replies first, then one slice); model that as one step per page.
        // Each step is an `update` in its own right and is held to the same
        // budget as a page (M6 task 3).
        if state.rebuild_running() {
            let stage = state.job.as_ref().map(|j| j.stage_name());
            let started = Instant::now();
            let (next, _cmds) = update(state, Msg::RebuildStep);
            let step = started.elapsed();
            state = next;
            steps_run += 1;
            if step > Duration::from_millis(16) {
                println!("  slow job step at page {page}: {step:?} stage {stage:?}");
            }
            page_times.push(step);
            if step > worst_page_time {
                worst_page_time = step;
                worst_page_index = page;
            }
        }
        pages_done = page + 1;
        if run_started.elapsed() > WALL_CAP {
            break;
        }
    }
    // The scan ends: `ScanComplete` leaves the view whole, which on a large
    // keyspace is one more job; step it to the swap, timing each update.
    let end_started = Instant::now();
    let (next, _cmds) = update(state, Msg::ScanComplete);
    state = next;
    let end_update = end_started.elapsed();
    page_times.push(end_update);
    worst_page_time = worst_page_time.max(end_update);
    while state.rebuild_running() {
        let started = Instant::now();
        let (next, _cmds) = update(state, Msg::RebuildStep);
        let step = started.elapsed();
        state = next;
        steps_run += 1;
        page_times.push(step);
        worst_page_time = worst_page_time.max(step);
    }
    let to_level = end_started.elapsed();
    println!(
        "[{}] whole-scan fold: scan end to a level list {to_level:?}, {steps_run} job steps run in all, final list covers {} of {} keys",
        fx.label(),
        state.list.covered_len(),
        state.keys.len()
    );
    let total = run_started.elapsed();
    let capped = pages_done < TOTAL_PAGES;
    let slow_pages = page_times
        .iter()
        .filter(|d| **d > Duration::from_millis(16))
        .count();
    let median_page_time = median(page_times);

    println!(
        "[{}] whole-scan fold, tree mode: {pages_done}/{TOTAL_PAGES} pages ({} keys loaded), total {total:?}, median page {median_page_time:?}, worst page #{worst_page_index} (at {} keys) took {worst_page_time:?}, {slow_pages} pages over 16ms{}",
        fx.label(),
        pages_done * 500,
        (worst_page_index + 1) * 500,
        if capped {
            " [wall-cap tripped — partial run]"
        } else {
            ""
        }
    );
    if capped {
        let extrapolated = total.mul_f64(TOTAL_PAGES as f64 / pages_done as f64);
        println!(
            "extrapolated full 1M-key scan total (LINEAR extrapolation from {pages_done} pages \
             — optimistic if per-page cost is actually superlinear in keyspace size): {extrapolated:?}"
        );
    }

    // The total must stay well inside the wall gate; the median page is AT
    // TARGET (most pages are free); the worst page is a CEILING (PRD §7
    // target 16ms) whose value is set per fixture.
    assert!(
        total < Duration::from_secs(budget.total_s),
        "whole scan took {total:?}, gate is {}s",
        budget.total_s
    );
    assert_budget("worst scan page", worst_page_time, budget.worst_ms);
    assert!(
        median_page_time < Duration::from_millis(16),
        "median page took {median_page_time:?}, budget is 16ms"
    );
}

#[test]
#[ignore]
fn whole_scan_fold_in_tree_mode_from_empty() {
    // Sorted control. Before M4 task 3 this took ~120s; ~0.4s now. Worst
    // update (a page or a job step): 2.3ms local, 3.9ms CI; AT TARGET 16ms
    // since M6 task 3. Total gate 2s since M6 task 5: 242-345ms on CI, 1.5x
    // the slowest rounded up to a whole second.
    whole_scan_fold(
        Fixture::Sorted,
        ScanBudget {
            worst_ms: 16,
            total_s: 2,
        },
    );
}

#[test]
#[ignore]
fn whole_scan_fold_in_tree_mode_from_empty_random_deep() {
    // AT TARGET since M6 task 3: the worst update, page or job step, was up to
    // 479ms before, 9.4ms local and 9.2ms CI after task 3, and 1.8ms local,
    // 2.9ms CI after task 5, with no update over 16ms. Total gate since M6 task 5: 548-825ms on CI (worst page 2.4-2.9ms),
    // x 1.5 = 1.24s, rounded up to a whole second = 2s. Was 6s (3.55s on CI).
    whole_scan_fold(
        Fixture::RandomDeep,
        ScanBudget {
            worst_ms: 16,
            total_s: 2,
        },
    );
}

// ── render alone, at 1M ───────────────────────────────────────────────────

fn render_alone(fx: Fixture, ceil_ms: u64) {
    // Companion to `golden.rs`'s `rendering_cost_does_not_grow_with_the_keyspace`
    // (200k keys, debug profile, the fast tripwire); this is the 1M-release
    // number PRD §7 names.
    let base = big_state(fx);
    let mut samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let started = Instant::now();
        render_once(&base);
        samples.push(started.elapsed());
    }
    let elapsed = median(samples);
    println!("[{}] render alone @ 1M keys: {elapsed:?}", fx.label());
    // AT TARGET: R2.6, PRD §7 — render cost is a function of viewport.
    assert_budget("render", elapsed, ceil_ms);
}
variants!(render_alone:
    render_alone_at_1m_keys => (Fixture::Sorted, 16),
    render_alone_at_1m_keys_random_deep => (Fixture::RandomDeep, 16));

// ── memory accounting: LoadedSet + View + Tree at 1M keys ────────────────

fn memory_budget(fx: Fixture, _unused: u64) {
    // Extends `loaded.rs`'s `a_million_keys_fit_inside_the_budget` to the two
    // structures beside the arena in a real browse: `KeyView`'s order index
    // and `Tree`'s folded rows (R2.3), built with tree mode on (an unbuilt
    // `Tree` is empty and would pass by omission). Pure arithmetic, no RSS.
    let mut state = State {
        keys: fx.build_keys(),
        list: KeyView::new("", FilterMode::Glob, SortBy::Scan),
        tree_mode: true,
        ..base_state()
    };
    state.rebuild_list();

    let keys_bytes = state.keys.heap_bytes();
    let view_bytes = state.list.heap_bytes();
    let tree_bytes = state.tree.heap_bytes();
    let total = keys_bytes + view_bytes + tree_bytes;

    println!(
        "[{}] LoadedSet {:.1}MB + View {:.1}MB + Tree {:.1}MB = {:.1}MB @ 1M keys",
        fx.label(),
        keys_bytes as f64 / 1024.0 / 1024.0,
        view_bytes as f64 / 1024.0 / 1024.0,
        tree_bytes as f64 / 1024.0 / 1024.0,
        total as f64 / 1024.0 / 1024.0,
    );

    // AT TARGET: PRD §7's 250MB RSS budget, R2.6/ADR-0010. Not RSS, so held
    // to half the budget, as `a_million_keys_fit_inside_the_budget` does.
    let budget = 250 * 1024 * 1024;
    assert!(
        total < budget / 2,
        "LoadedSet+View+Tree took {}MB; half the 250MB budget is {}MB",
        total / 1024 / 1024,
        budget / 2 / 1024 / 1024
    );
}
variants!(memory_budget:
    loaded_set_plus_view_plus_tree_fit_inside_the_budget => (Fixture::Sorted, 0),
    loaded_set_plus_view_plus_tree_fit_inside_the_budget_random_deep => (Fixture::RandomDeep, 0));

// ── the rebuild job: slice calibration and memory while it runs ─────────

fn heap_total(state: &State) -> usize {
    state.keys.heap_bytes() + state.list.heap_bytes() + state.tree.heap_bytes()
}

/// Peak heap while the heaviest job runs (tree toggle from scan order: filter,
/// merge sort with its scratch, inverse and fold, new buffers beside the old
/// ones), against the 250MB budget (decision 9).
fn job_memory_peak(fx: Fixture, _unused: u64) {
    let (mut state, _) = update(big_state(fx), key(KeyCode::Char('t')));
    assert!(state.rebuild_running());
    let baseline = heap_total(&state);
    let (mut peak_job, mut steps) = (0usize, 0usize);
    while let Some(job) = state.job.as_ref() {
        peak_job = peak_job.max(job.heap_bytes());
        steps += 1;
        let (next, _) = update(state, Msg::RebuildStep);
        state = next;
    }
    let mb = |b: usize| b as f64 / 1024.0 / 1024.0;
    println!(
        "[{}] job memory @ 1M keys: shown list {:.1}MB (LoadedSet+View+Tree) + job buffers peak {:.1}MB = {:.1}MB during the job; after the swap {:.1}MB; {steps} steps",
        fx.label(),
        mb(baseline),
        mb(peak_job),
        mb(baseline + peak_job),
        mb(heap_total(&state)),
    );
    // AT TARGET: PRD §7's 250MB RSS budget. Arithmetic, not RSS (the RSS
    // figure above also carries the fixture's cached copy), so held to the
    // budget itself rather than half of it.
    assert!(
        baseline + peak_job < 250 * 1024 * 1024,
        "heap during a rebuild job {:.1}MB exceeds the 250MB budget",
        mb(baseline + peak_job)
    );
}
variants!(job_memory_peak:
    job_memory_peak_random_deep => (Fixture::RandomDeep, 0));

/// Not a gate: how the worst step of each stage moves with the slice, to pick
/// `REBUILD_SLICE` (the Outcome of `docs/plans/m6-rebuild-job.md` records the
/// numbers). Run with `--nocapture`.
#[test]
#[ignore]
fn rebuild_slice_calibration_random_deep() {
    for slice in [8_192usize, 16_384, 24_576, 32_768, 49_152, 65_536] {
        let mut base = big_state(Fixture::RandomDeep);
        base.rebuild_slice = Some(slice);
        println!("slice {slice}:");
        profile(&base, 3, || key(KeyCode::Char('t')));
    }
}
