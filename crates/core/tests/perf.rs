//! Release-mode performance budgets (M4 task 2, `docs/plans/m4-perf-harness.md`).
//!
//! Every test here is `#[ignore]`d: `cargo test --workspace` stays debug-mode
//! and Docker-free. These run via a dedicated CI job, in release, the same
//! shape as the Docker-backed integration suite already uses:
//!
//! ```text
//! cargo test -p redis-pane-core --release --test perf -- --ignored --nocapture
//! ```
//!
//! **The override that shapes every budget below:** a budget the current code
//! does not yet meet must not turn CI red. Tasks 3–4 (`scan_batch`,
//! `rebuild_list`'s hot paths) fix the slow cases; this task only measures
//! and gates against a named constant. Where the current code already meets
//! the PRD §7 target, the constant *is* the target, labelled `AT TARGET`.
//! Where it does not, the constant is a **regression ceiling** — the
//! measured local baseline × 1.5, rounded up — labelled `CEILING`, with the
//! PRD target named in a comment and "tightened by M4 task 3/4". Every test
//! prints its measured number first, so a CI log carries the real value even
//! though the assertion is against the (looser) ceiling.
//!
//! Keys are realistic: `user:{i:08}:session`, the same generator
//! `crates/core/examples/memreport.rs` already uses, with two `:` separators
//! so tree mode (R2.3) actually has something to fold.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use redis_pane_core::clock::FixedClock;
use redis_pane_core::msg::{KeyCode, KeyPress};
use redis_pane_core::render;
use redis_pane_core::state::{
    Connection, Environment, FilterMode, KeyView, Link, LoadedSet, SortBy, Source, State, Tracking,
};
use redis_pane_core::theme::{ColorDepth, Theme};
use redis_pane_core::{Msg, update};

const COLS: u16 = 130;
const ROWS: u16 = 26;

fn key_name(i: usize) -> String {
    format!("user:{i:08}:session")
}

/// A deterministic, columnar `State` with `n` keys already scanned and the
/// list rebuilt, in the given view mode — the steady state a browse sits in
/// once a scan has finished, which is what every per-keystroke test below
/// perturbs with one message. `tree_mode` and `sort` are independent knobs:
/// tree mode forces a Name sort (`rebuild_list`'s own invariant) regardless
/// of what `sort` asks for, so passing `SortBy::Scan` with `tree_mode: true`
/// still lands on Name — exactly what the real app does the moment Tree mode
/// (the default view) turns on.
fn make_state(n: usize, tree_mode: bool, sort: SortBy) -> State {
    let mut keys = LoadedSet::default();
    for i in 0..n {
        assert!(keys.push(key_name(i).as_bytes()));
    }
    let mut state = State {
        cols: COLS,
        rows: ROWS,
        connection: Connection {
            target: "perf-harness:6379/0".into(),
            environment: Environment::Staging,
            source: Source::Profile("perf".into()),
        },
        last_read_ms: Some(0),
        link: Link::Up {
            version: "8.4.0".into(),
            tracking: Tracking::Armed,
        },
        keys,
        list: KeyView::new("", FilterMode::Glob, sort),
        tree_mode,
        ..State::default()
    };
    state.rebuild_list();
    state
}

/// Flat view, scan order — the shape every pre-existing test below uses,
/// kept as its own name since most of these tests are about a keystroke's
/// cost in that baseline view, not about the view itself.
fn big_state(n: usize) -> State {
    make_state(n, false, SortBy::Scan)
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

// ── (a) scroll keystroke ─────────────────────────────────────────────────

#[test]
#[ignore]
fn scroll_keystroke_at_1m_keys() {
    let base = big_state(1_000_000);
    let elapsed = time_update_and_render(&base, 11, || Msg::Key(KeyPress::plain(KeyCode::Down)));
    println!("scroll keystroke @ 1M keys: {elapsed:?}");
    // AT TARGET: PRD §7 — a keystroke must be answerable in one frame (16ms).
    // Moving the selection touches no arena, no sort, no rebuild; only the
    // render cost scales with the keyspace, and that is already bounded by
    // virtualization (R2.6).
    assert!(
        elapsed < Duration::from_millis(16),
        "scroll took {elapsed:?}, budget is 16ms"
    );
}

// ── (b) filter keystroke ─────────────────────────────────────────────────

#[test]
#[ignore]
fn filter_keystroke_at_1m_keys() {
    let mut base = big_state(1_000_000);
    // Mid-typing: a filter is already active and capturing, and one more
    // character arrives. `KeyView::rebuild` scans the *whole* Loaded set on
    // every keystroke (ADR-0010's `rebuild` doc comment: "cheap enough to
    // run on every change") — this is task 3's hot path, measured here, not
    // fixed here.
    base.list.filter = "user:0000".into();
    base.filtering = true;
    base.rebuild_list();
    let elapsed =
        time_update_and_render(&base, 11, || Msg::Key(KeyPress::plain(KeyCode::Char('1'))));
    println!("filter keystroke @ 1M keys: {elapsed:?}");
    // CEILING: PRD §7 target is 16ms; a full-keyspace rescan on every
    // character is exactly what task 3 is scoped to fix. Local baseline
    // measured ~22.3ms, but CI's ubuntu runner measured 31.2ms, so the
    // ceiling is the slower machine's baseline × 1.5, rounded up: 47ms.
    // Tightened by M4 task 3.
    assert!(
        elapsed < Duration::from_millis(47),
        "filter keystroke took {elapsed:?}, ceiling is 47ms (target 16ms, M4 task 3)"
    );
}

// ── (c) sort change ──────────────────────────────────────────────────────

#[test]
#[ignore]
fn sort_change_at_1m_keys() {
    let base = big_state(1_000_000);
    let elapsed =
        time_update_and_render(&base, 11, || Msg::Key(KeyPress::plain(KeyCode::Char('s'))));
    println!("sort change (scan -> name) @ 1M keys: {elapsed:?}");
    // AT TARGET: PRD §7's 16ms — sorting 1M keys by name (an O(n log n)
    // byte-slice comparison sort over the index vector, never the arena
    // itself, ADR-0010) already lands at ~3.8ms locally.
    assert!(
        elapsed < Duration::from_millis(16),
        "sort change took {elapsed:?}, budget is 16ms"
    );
}

// ── (d) toggling tree mode ───────────────────────────────────────────────

#[test]
#[ignore]
fn toggle_tree_at_1m_keys() {
    let base = big_state(1_000_000);
    let elapsed =
        time_update_and_render(&base, 11, || Msg::Key(KeyPress::plain(KeyCode::Char('t'))));
    println!("toggle tree mode @ 1M keys: {elapsed:?}");
    // CEILING: PRD §7 target is 16ms; entering tree mode forces a Name sort
    // (rebuild_list's invariant) *and* a one-pass `Tree::rebuild` over all 1M
    // rows — by far the most expensive case measured here. Local baseline
    // measured ~129ms; ceiling = baseline × 1.5, rounded up to 194ms.
    // Tightened by M4 task 4 (Tree's own representation).
    assert!(
        elapsed < Duration::from_millis(194),
        "tree toggle took {elapsed:?}, ceiling is 194ms (target 16ms, M4 task 4)"
    );
}

// ── (e) one ScanBatch page arriving into an already-1M-key set ──────────

#[test]
#[ignore]
fn scan_batch_of_500_into_1m_keys() {
    let base = big_state(1_000_000);
    let page: Vec<Vec<u8>> = (1_000_000..1_000_500)
        .map(|i| key_name(i).into_bytes())
        .collect();
    let elapsed = time_update_and_render(&base, 11, || Msg::ScanBatch { keys: page.clone() });
    println!("ScanBatch of 500 into 1M keys: {elapsed:?}");
    // AT TARGET: this is task 3's named hot path — `scan_batch` calls
    // `rebuild_list` on every batch, so each page pays for a full-keyspace
    // rebuild rather than an incremental append — but at 1M keys that full
    // rebuild already lands at ~2.3ms locally, comfortably inside PLAN's
    // "update + render at 1M keys within 16ms" bar every per-keystroke
    // update is held to.
    assert!(
        elapsed < Duration::from_millis(16),
        "ScanBatch(500) into 1M took {elapsed:?}, budget is 16ms"
    );
}

// ── (e2) one ScanBatch page, tree mode (the default view) ───────────────

#[test]
#[ignore]
fn scan_batch_of_500_into_1m_keys_tree_mode() {
    // Tree mode is the app's *default* view (DESIGN §1), and `rebuild_list`
    // in tree mode forces a Name sort plus a full `Tree::rebuild` on every
    // `ScanBatch`, not only on the final one — so the flat/scan-order case
    // above, while a real data point, is not the number a fresh launch
    // actually pays per page. This is.
    let base = make_state(1_000_000, true, SortBy::Scan);
    let page: Vec<Vec<u8>> = (1_000_000..1_000_500)
        .map(|i| key_name(i).into_bytes())
        .collect();
    let elapsed = time_update_and_render(&base, 11, || Msg::ScanBatch { keys: page.clone() });
    println!("ScanBatch of 500 into 1M keys, tree mode: {elapsed:?}");
    // CEILING: PRD §7 target is 16ms; every page in tree mode pays for a
    // full-keyspace Name sort *and* a full `Tree::rebuild`, which is why
    // this is far more expensive than the flat/scan-order case just above.
    // Local baseline measured ~130ms; ceiling = baseline × 1.5, rounded up
    // to 195ms. Tightened by M4 task 3 (scan_batch/rebuild_list) and task 4
    // (Tree's own representation).
    assert!(
        elapsed < Duration::from_millis(195),
        "ScanBatch(500) into 1M (tree mode) took {elapsed:?}, ceiling is 195ms (target 16ms, M4 task 3/4)"
    );
}

// ── (e3) one ScanBatch page, flat view sorted by name ───────────────────

#[test]
#[ignore]
fn scan_batch_of_500_into_1m_keys_flat_sorted_by_name() {
    // The middle case between the cheap scan-order default and the
    // expensive tree-mode default: flat (no folding), but already sorted by
    // Name rather than Scan order, so every page still pays for a full
    // re-sort — just not the `Tree::rebuild` on top of it.
    let base = make_state(1_000_000, false, SortBy::Name);
    let page: Vec<Vec<u8>> = (1_000_000..1_000_500)
        .map(|i| key_name(i).into_bytes())
        .collect();
    let elapsed = time_update_and_render(&base, 11, || Msg::ScanBatch { keys: page.clone() });
    println!("ScanBatch of 500 into 1M keys, flat sorted by name: {elapsed:?}");
    // AT TARGET: PRD §7's 16ms. A full re-sort by name over the index
    // vector on every page (never touching the arena, ADR-0010) already
    // lands at ~7ms locally — the `Tree::rebuild` in the tree-mode case
    // above is what actually blows the budget, not the re-sort by itself.
    assert!(
        elapsed < Duration::from_millis(16),
        "ScanBatch(500) into 1M (flat, sorted by name) took {elapsed:?}, budget is 16ms"
    );
}

// ── whole scan, folded page by page, in tree mode (the default view) ────

#[test]
#[ignore]
fn whole_scan_fold_in_tree_mode_from_empty() {
    // Tree mode is the default view, so this is what a fresh launch against
    // a 1M-key server actually does: 2,000 pages of 500 keys each, folded
    // one at a time via `update(state, Msg::ScanBatch { .. })`, exactly as
    // `crates/app/src/update/scan.rs`'s real dispatch loop does it. The
    // isolated `scan_batch_of_500_into_1m_keys_tree_mode` case above times
    // one page *at* 1M keys; this one times every page *on the way there*,
    // which is the only way to see whether the per-page cost stays flat
    // (true O(n) total work) or grows with the keyspace already loaded
    // (O(n²) total, since tree mode rebuilds proportional to the *current*
    // size on every page, not just the page that just arrived).
    //
    // Capped by wall time, not by page count: if per-page cost grows with
    // keyspace size (plausible — Name-sorting and tree-rebuilding a larger
    // index on every page), running all 2,000 pages could take minutes even
    // in release, which would make this test the thing it is trying to
    // avoid: a number nobody can afford to generate in CI. If the cap
    // trips, the loop stops early, reports how far it got, and prints a
    // *linear* extrapolation of the full-scan total from the completed
    // pages — likely optimistic if the real cost is superlinear, which is
    // called out explicitly in the printed line rather than asserted on.
    const TOTAL_PAGES: usize = 2_000; // 2,000 × 500 = 1,000,000 keys
    const WALL_CAP: Duration = Duration::from_secs(240);

    let mut state = State {
        cols: COLS,
        rows: ROWS,
        connection: Connection {
            target: "perf-harness:6379/0".into(),
            environment: Environment::Staging,
            source: Source::Profile("perf".into()),
        },
        last_read_ms: Some(0),
        link: Link::Up {
            version: "8.4.0".into(),
            tracking: Tracking::Armed,
        },
        list: KeyView::new("", FilterMode::Glob, SortBy::Scan),
        tree_mode: true,
        ..State::default()
    };

    let run_started = Instant::now();
    let mut worst_page_time = Duration::ZERO;
    let mut worst_page_index = 0usize;
    let mut pages_done = 0usize;
    for page in 0..TOTAL_PAGES {
        let batch: Vec<Vec<u8>> = (page * 500..page * 500 + 500)
            .map(|i| key_name(i).into_bytes())
            .collect();
        let started = Instant::now();
        let (next, _cmds) = update(state, Msg::ScanBatch { keys: batch });
        let elapsed = started.elapsed();
        state = next;
        if elapsed > worst_page_time {
            worst_page_time = elapsed;
            worst_page_index = page;
        }
        pages_done = page + 1;
        if run_started.elapsed() > WALL_CAP {
            break;
        }
    }
    let total = run_started.elapsed();
    let capped = pages_done < TOTAL_PAGES;

    println!(
        "whole-scan fold, tree mode: {pages_done}/{TOTAL_PAGES} pages ({} keys loaded), total {total:?}, worst page #{worst_page_index} (at {} keys) took {worst_page_time:?}{}",
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

    // CEILING on the worst single page: PRD §7's target is 16ms (one
    // frame) — not remotely met once tree mode is rebuilding a
    // near-1M-key index on every page, which is exactly what this test
    // exists to surface. Local baseline (full, uncapped run) measured a
    // worst page of ~251ms; ceiling = baseline × 1.5, rounded up to 378ms.
    // Tightened by M4 task 3/4; this is the number that should move the
    // most once they land.
    assert!(
        worst_page_time < Duration::from_millis(378),
        "worst page took {worst_page_time:?}, ceiling is 378ms (target 16ms, M4 task 3/4)"
    );
}

// ── render alone, at 1M ───────────────────────────────────────────────────

#[test]
#[ignore]
fn render_alone_at_1m_keys() {
    // Companion to `golden.rs`'s `rendering_cost_does_not_grow_with_the_keyspace`,
    // which proves the same shape at 200k keys in the **debug** profile (the
    // default `cargo test` run). That test stays: it is Docker-free, runs on
    // every push, and catches a virtualization regression immediately,
    // whereas this one needs `--release` and `--ignored` and only runs on
    // the dedicated perf job. Neither supersedes the other — 200k-debug is
    // the fast tripwire, 1M-release is the number PRD §7 actually names.
    let base = big_state(1_000_000);
    let mut samples = Vec::with_capacity(50);
    for _ in 0..50 {
        let started = Instant::now();
        render_once(&base);
        samples.push(started.elapsed());
    }
    let elapsed = median(samples);
    println!("render alone @ 1M keys: {elapsed:?}");
    // AT TARGET: R2.6, PRD §7 — a frame must be drawable in 16ms regardless
    // of keyspace size; the list is virtualized (render cost is a function
    // of viewport, not keyspace).
    assert!(
        elapsed < Duration::from_millis(16),
        "render took {elapsed:?}, budget is 16ms"
    );
}

// ── memory accounting: LoadedSet + View + Tree at 1M keys ────────────────

#[test]
#[ignore]
fn loaded_set_plus_view_plus_tree_fit_inside_the_budget() {
    // Extends `loaded.rs`'s `a_million_keys_fit_inside_the_budget`, which
    // only accounts for the `LoadedSet` arena itself, to the two structures
    // that sit beside it in a real browse: the `View`'s order index (ADR-0010
    // already budgets ~15MB of indices at 1M keys, this is the part of that
    // which lives in `KeyView`) and `Tree`'s folded rows (R2.3) — built here
    // with tree mode on, since an unbuilt `Tree` is empty and would make this
    // test pass by omission. Pure arithmetic, no RSS measurement, so it is
    // deterministic cross-machine and does not need `--release` to be fast
    // enough — ignored anyway, to keep it with the rest of this file's
    // 1M-key fixtures rather than splitting the harness across two files.
    let mut state = big_state(1_000_000);
    state.tree_mode = true;
    state.rebuild_list();

    let keys_bytes = state.keys.heap_bytes();
    let view_bytes = state.list.heap_bytes();
    let tree_bytes = state.tree.heap_bytes();
    let total = keys_bytes + view_bytes + tree_bytes;

    println!(
        "LoadedSet {:.1}MB + View {:.1}MB + Tree {:.1}MB = {:.1}MB @ 1M keys",
        keys_bytes as f64 / 1024.0 / 1024.0,
        view_bytes as f64 / 1024.0 / 1024.0,
        tree_bytes as f64 / 1024.0 / 1024.0,
        total as f64 / 1024.0 / 1024.0,
    );

    // AT TARGET: PRD §7's 250MB RSS budget, R2.6/ADR-0010. This pure
    // arithmetic total is not RSS (no allocator overhead, no runtime, no
    // Viewer), so it is held to half the budget, exactly as
    // `a_million_keys_fit_inside_the_budget` already does, leaving room for
    // the rest of the process.
    let budget = 250 * 1024 * 1024;
    assert!(
        total < budget / 2,
        "LoadedSet+View+Tree took {}MB; half the 250MB budget is {}MB",
        total / 1024 / 1024,
        budget / 2 / 1024 / 1024
    );
}
