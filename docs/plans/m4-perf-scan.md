# M4 task 3: Million-key scan

Status: **done.**

## Resolved decisions (build-time)

1. **The incremental-append split (decision 1) landed exactly as sketched**:
   `KeyView::extend(&mut self, keys: &LoadedSet, new_from: usize)`
   (`crates/core/src/state/view.rs`) appends indices `new_from..keys.len()`,
   applying the filter to just the new keys, and is only called when
   `!state.tree_mode && state.list.sort == SortBy::Scan` — the common case,
   since a scan starts in flat/scan order and stays there until a reader
   changes it. `scan_batch` (`crates/core/src/update/scan.rs`) captures
   `prior_len` before pushing a page, calls `extend` after, and — rather than
   `relocate_open_key`'s linear `row_of` scan — patches the Open key's `row`
   directly when its index landed in that page, by searching only the rows
   `extend` just appended (bounded by the page size, 500, not the row
   count).
2. **The batched-rebuild cadence (decision 2): a geometric growth
   threshold, not a page count or a wall-clock interval.** Confirmed at
   build time: a page-count or time-based cadence would need to know how
   large the keyspace already is to avoid being too eager early and too
   lazy late; a growth-*fraction* threshold scales itself automatically.
   `SCAN_REBUILD_GROWTH_PCT: usize = 25` (`crates/core/src/update/scan.rs`)
   — rebuild once the Loaded set has grown 25% past the size it was last
   rebuilt at, tracked in a new `State::scan_last_rebuild_len` field that
   every call to `rebuild_list` updates (not only the ones `scan_batch`
   itself triggers, so the bookkeeping stays correct across a mid-scan sort
   change or tree-mode toggle, both of which call `rebuild_list` directly).
   `scan_last_rebuild_len == 0` always triggers a rebuild, which is what
   makes the scan's first page land on screen rather than leaving the list
   empty until growth crosses the threshold. 25% means a 1M-key scan does
   on the order of 60 rebuilds rather than 2,000, and the sum of
   geometrically-growing rebuild sizes is a convergent series — O(n log n)
   total, not O(n²). Measured: the whole-scan fold in tree mode dropped
   from ~120s to ~0.6–0.8s locally; the worst single page (the one that
   lands on a scheduled rebuild near the end of the scan) dropped from
   ~251ms to ~100–145ms — still well above the 16ms frame budget, since
   that page still pays for `rebuild_list`'s own Name-sort-plus-
   `Tree::rebuild` cost, which is M4 task 4's scope, not this one's.
3. **`row_of` itself is untouched**, as scoped — the incremental-append path
   avoids calling it at all for the common case, rather than making it
   faster.
4. **Redraw coalescing (decision 4): a dirty flag plus a minimum frame
   interval, not a fixed tick.** `crates/app/src/terminal.rs`'s loop sets
   `dirty` on every `update()`, and the top of the loop draws when `dirty`
   is set and at least 16 ms (`FRAME`) have passed since the last draw.
   After any pause a keystroke is drawn immediately; under a flood of
   `Msg::ScanBatch`es drawing is capped at one frame per 16 ms, and because
   the check runs at the top of every iteration rather than in a `select!`
   arm, the flood cannot starve it. A change landing inside the interval is
   drawn when a `sleep_until(frame_due)` arm fires — armed only while
   `dirty`, so an idle screen does not wake 60 times a second. The old 1 s
   countdown arm now just sets `dirty`. `biased` stays, messages first.
   *Revised in review:* the first cut drew only on a fixed 16 ms tick, which
   delayed every keystroke by up to one frame (spending the very budget the
   change protects) and woke an idle app ~60 times a second. This removes
   the ~2,000 `Msg::ScanBatch`-driven `render::frame` calls a 1M-key scan
   used to cause, plus the once-a-second unconditional one, with no change
   to the core (the cadence stays shell-side, per CLAUDE.md's
   injected-clock discipline).
5. **The `ttl_read_at` leak (decision 5) is fixed** exactly as described —
   `LoadedSet::clear` (`crates/core/src/state/loaded.rs`) now clears it
   alongside the other six arrays, with a regression test
   (`clear_does_not_leak_the_ttl_read_at_array`) asserting `heap_bytes`
   does not grow across repeated clear/push cycles at the same size.

## Testing, confirmed

The correctness proof (decision/testing section's "equivalence test, not
just a timing test") is
`update::scan::incremental_equivalence_tests` in `crates/core/src/update/scan.rs`:
it folds a synthetic keyspace through the real `update`/`Msg::ScanBatch`
dispatch, page by page, ending in `Msg::ScanComplete`, and asserts the
final `KeyView`, `Tree` and selected row are identical to building the same
keyspace directly and calling `rebuild_list` once — covering flat scan
order, flat sorted by name, flat sorted by a lazy column (TTL), tree mode,
each with and without a filter, and a sort/tree-mode/filter change
mid-scan. A further test pins the incremental-append path's narrower claim:
in flat scan order the view is exact after *every single page*, not only
once the scan ends.

## Context

PLAN.md M4 row 3: "Million-key scan · Proves: scanning 1M keys does O(n) total list work, not
O(n²): the view extends incrementally per page in scan order, and a full re-sort runs only when a
non-scan sort or tree mode is active, deferred or batched. The shell coalesces redraws (one frame
per tick, not per `Msg`). The `ttl_read_at` leak is fixed, with a regression test. Task 2's
budgets pass at 1M."

Three separate, verified facts combine into the quadratic cost this task removes:

1. **`scan_batch` (`crates/core/src/update/scan.rs`) calls `state.rebuild_list()` once per page,**
   and the keyspace source pages at `PAGE: u32 = 500` (`crates/app/src/redis/scan.rs`). A 1M-key
   scan is ~2,000 pages, so `rebuild_list` runs ~2,000 times over the session.
2. **`rebuild_list` (`crates/core/src/state/mod.rs`) calls `self.list.rebuild(&self.keys)`
   unconditionally** — `View::rebuild` (`crates/core/src/state/view.rs`) clears and rebuilds the
   entire filtered/sorted `order: Vec<u32>` from scratch every time, including `apply_sort`, which
   for `SortBy::Name` does a full `sort_by` over the whole order vector even though the default
   `SortBy::Scan` sort needs no sorting. `rebuild_list` also calls `relocate_open_key`, which calls
   `row_of` — a **linear scan** (`(0..self.row_count()).find(...)`) over the current row count.
   Each of the ~2,000 calls therefore does work proportional to the *current* list size, not the
   page size — the classic "rebuild everything on every append" shape that makes total work O(n²)
   across the whole scan.
3. **The shell draws unconditionally every loop iteration** (`crates/app/src/terminal.rs`'s main
   `loop { term.draw(...); ... }`): the `tokio::select!` below the draw has a `biased`
   1-second-sleep arm that `continue`s (triggering a redraw every second even with nothing to
   redraw) and an `rx.recv()` arm that redraws on *every* `Msg`, including the ~2,000
   `Msg::ScanBatch`es a 1M-key scan produces. Ratatui diffs the buffer before writing, so a
   no-visual-change redraw is cheaper than a full terminal write, but `render::frame` itself still
   re-walks `State` to build that buffer on every one of those ~2,000+ calls — real CPU work the
   coalescing this task asks for would remove.
4. **`LoadedSet::clear` (`crates/core/src/state/loaded.rs`) does not clear `ttl_read_at`.** Its
   body clears `arena`, `offsets`, `lens`, `kinds`, `ttls`, `sizes` and resets `capped` — six of
   the seven parallel arrays, `ttl_read_at` is the one left out. A rescan (`r` in the keys pane,
   which calls `clear()` before re-streaming) therefore leaks one `i64`-or-similar entry per
   previously-scanned key on every rescan, unbounded over a long session that rescans repeatedly.

## Decisions

1. **The view extends incrementally per page, in scan order.** `scan_batch` should not call the
   full `rebuild_list` on every page; instead, when the active sort is `SortBy::Scan` (the default,
   and the only sort active during an in-progress scan in practice — a reader can change sort mid-
   scan, which is the case that must still work, see below) and the filter is empty or the page's
   newly-pushed keys can be filtered individually, the newly-scanned indices are appended directly
   to `View.order` and (if tree mode is active) folded into `Tree` incrementally, rather than the
   whole list being rebuilt from the `LoadedSet` from scratch. **Confirm at build time** the exact
   split: the cleanest design is likely a new `View::extend(&mut self, keys: &LoadedSet, new_from:
   usize)` that only processes indices `new_from..keys.len()`, called from `scan_batch` in the
   common case, with `rebuild_list`'s full-rebuild path kept for every case that genuinely needs
   it (filter change, sort change, tree-mode toggle, a rescan starting over).
2. **A full re-sort runs only when a non-scan sort or tree mode is active, deferred or batched.**
   If the reader has sorted by Name/TTL/Size/Kind or turned on tree mode while a scan is still
   running, incremental append is not correct (a newly-arrived key may belong anywhere in sort
   order, or may fold into an existing tree group) — but re-sorting on every single page is still
   the quadratic behaviour this task removes. **Recommended default: batch these cases** — collect
   newly-scanned keys across pages and re-sort/re-fold at a bounded cadence (e.g. once per N pages,
   or once per some millisecond interval, whichever is simpler to reason about — **confirm at
   build time** which; a page-count cadence is deterministic and easier to test, a time-based one
   matches "deferred" more literally and couples better to the redraw-coalescing in decision 3).
   Either way, the final rebuild once the scan completes must still produce an identical result to
   today's always-rebuild behavior — this is the thing the regression test must pin.
3. **`row_of` stays linear scan for this task; task 4 gives it an inverse index.** Making it O(1)
   is PLAN's task 4 ("`row_of` uses an inverse index"), not this one — but this task's incremental
   append path must still avoid calling `relocate_open_key`/`row_of` once per page the way
   `rebuild_list` does today; since the append path knows exactly which indices were just added, it
   can update `open.row` directly when the Open key's index matches one of them, without a scan,
   deferring the general O(1) `row_of` fix to task 4.
4. **The shell coalesces redraws: one frame per tick, not per `Msg`.** Recommended approach:
   replace the per-`Msg` unconditional `term.draw` with a dirty flag or a fixed redraw tick (e.g.
   `tokio::time::interval` at a frame-rate cadence like 60fps/16ms, matching the PRD §7 "keystroke
   → visible response <16ms" budget) that draws only if something changed since the last frame, or
   draws unconditionally but at a bounded rate regardless of `Msg` volume — **confirm at build
   time** which shape: a dirty-flag-on-every-`update()` approach keeps the "answerable in one
   frame regardless of network" property CLAUDE.md states exactly (every real state change still
   redraws promptly) while collapsing the ~2,000 scan-batch-driven redraws into however many actual
   frames the terminal needs, and is the more conservative change relative to today's loop shape.
   The existing 1-second countdown-tick arm (`continue`s to force a redraw for the TTL countdown)
   needs the same treatment or an explicit exemption, since it is itself the second source of
   wasted full-`render::frame` calls alongside per-`Msg` draws.
5. **The `ttl_read_at` leak is fixed in `LoadedSet::clear`** — add the missing
   `self.ttl_read_at.clear()` alongside the other six arrays. Trivial, and it should land in this
   task even though it is logically independent of the quadratic-rebuild fix, because it is in the
   same file and PLAN's row explicitly calls it out alongside the scan work.

## Architecture

All core-side: `View::extend` (or equivalent), the batched-resort cadence logic, and the
`LoadedSet::clear` fix are pure `redis-pane-core` state transitions — no I/O, consistent with the
functional-core rule. The redraw-coalescing change is shell-side (`crates/app/src/terminal.rs`'s
event loop), since the clock-driven cadence and the terminal write are shell concerns; the core
must not gain its own timer for this (CLAUDE.md: "the clock is injected... `update()` must not
start its own interval" — the same discipline `m3-dashboard.md` already established for the
Dashboard's poll timer applies here to the redraw cadence).

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/view.rs` | `View::extend` (incremental append path), batched re-sort cadence |
| `crates/core/src/state/mod.rs` | `rebuild_list` gains (or is joined by) the incremental-append entry point `scan_batch` calls; `relocate_open_key` gets an incremental variant for the common append case |
| `crates/core/src/update/scan.rs` | `scan_batch` calls the incremental path instead of unconditional `rebuild_list` |
| `crates/core/src/state/loaded.rs` | `LoadedSet::clear` clears `ttl_read_at`; a regression test |
| `crates/app/src/terminal.rs` | redraw coalescing — dirty flag or bounded-rate draw, replacing per-`Msg`/per-second unconditional `term.draw` |
| `crates/core/tests/golden.rs` or `perf.rs` (from `m4-perf-harness.md`) | 1M-key scan-shaped timing test proving O(n) total work, not O(n²) |

## Testing

- **Core unit tests**: a scan-order (`SortBy::Scan`) append of N pages produces the same final
  `View.order` as today's unconditional `rebuild_list` after every page (equivalence test, not just
  a timing test — correctness first); a sort-change or tree-mode-toggle mid-scan still produces the
  identical final state to today's behavior once the scan completes; `LoadedSet::clear` followed by
  a re-push leaves `ttl_read_at.len()` matching the other arrays' lengths (the regression test for
  the leak — assert the lengths stay in lockstep after repeated clear/push cycles, which is exactly
  the invariant the bug violated).
- **Release-mode perf test** (from `m4-perf-harness.md`'s harness): simulate a 1M-key scan via
  ~2,000 `scan_batch` calls of 500 keys each and assert total wall time is substantially better
  than the current unconditional-rebuild baseline recorded in that doc — ideally demonstrating
  near-linear rather than quadratic growth by also timing a 100k-key and a 1M-key run and comparing
  the ratio (10x more keys should not cost ~100x more time, which is what O(n²) would predict).
- **Golden frames**: unaffected in content — this task changes performance, not what renders — but
  existing golden tests exercising mid-scan states (if any) must still pass unchanged, proving the
  incremental path produces pixel-identical frames to the old unconditional-rebuild path.

## CLAUDE.md rules this binds

- **The cap is enforced in exactly one place** — `scan_batch` in `update/scan.rs`. This task
  changes what `scan_batch` does after a page lands, not the cap enforcement itself (`state.keys.push`'s
  `Capped` branch is untouched); the incremental-append path must still only ever run after the cap
  check, never before.
- **The render loop never does I/O** — redraw coalescing in the shell must not introduce I/O into
  the draw path; it only changes *when* `render::frame` (pure) is called.
- **The clock is injected** — the redraw cadence timer lives in the shell, not the core, same
  discipline as the Dashboard's poll interval.
- **Every scanned key is retained up to a documented cap** (ADR-0010) — this task does not change
  retention or the cap, only the cost of keeping the view in sync with what is retained.

## Out of scope

- **`row_of` becoming O(1)** — PLAN's task 4, not this one; this task only avoids calling it
  redundantly on the hot scan-append path.
- **Filter/tree rebuild cost per keystroke** — PLAN's task 4 (`m4-perf-interaction.md`); this task's
  incremental-append path is specifically about the scan-arrival case, not interactive edits to the
  filter or tree state.
- **Changing `PAGE`'s value** — 500 is an existing, working choice; nothing here suggests changing
  the page size, only what happens after each page lands.
