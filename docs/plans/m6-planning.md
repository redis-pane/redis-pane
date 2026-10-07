# M6: rebuilds at real SCAN order. Task plan

Status: **tasks 1-5 done** (PR #78, PR #79, PR #80, PR #81, PR #82; task 5 pending merge). The design and analysis are in
[`m6-perf-rebuild.md`](m6-perf-rebuild.md). This file splits it into tasks in the M5 shape: one
plan doc per task, one Sonnet subagent run per task, each merged before the next starts.

## Scope decisions (user, 2026-10-07)

- **Never freeze first.** Slicing rebuilds across frames (task 3) comes before the sort and fold
  speedups (tasks 4 and 5).
- **While a rebuild runs, the old list stays on screen and usable**, with a `rebuilding N%`
  readout and a single swap at the end.

## Task table

| # | Task (plan doc) | Proves |
|---|---|---|
| 1 | Honest harness ([`m6-harness.md`](m6-harness.md)) | `perf.rs` measures keys in random and arrival order, deep names, collapse, fold-only, the whole-scan worst page, and total time until the new list appears. Ceilings are re-baselined from CI. No product change |
| 2 | Fold-only rebuild ([`m6-fold-only.md`](m6-fold-only.md)) | Collapse and expand re-fold the existing name-ordered view, with no re-filter or re-sort, and so does entering tree mode when the view is already in Name order. Equivalence holds against `rebuild_list` |
| 3 | Sliced rebuild job ([`m6-rebuild-job.md`](m6-rebuild-job.md)) | No `update` exceeds 16 ms at 1M random-order keys. A large rebuild runs as a resumable `RebuildJob`, the old list stays usable with `rebuilding N%`, and the new list is swapped in. User actions replace a job; scan pages mark it dirty. Small rebuilds stay synchronous, so every existing test and golden is unchanged |
| 4 | Fast name sort ([`m6-fast-sort.md`](m6-fast-sort.md)) | Name sort at 1M deep random order takes ≤ 80 ms total, via a sort-prefix column and a record sort that refines ties by gathering. A measured go or no-go on a persistent name order is recorded |
| 5 | Fast fold ([`m6-fast-fold.md`](m6-fast-fold.md)) | The fold takes ≤ 60 ms total at 1M deep random order, using the sort's LCP plus a gather or prefetch pass. Tree toggle shows its list within 250 ms total |
| 6 | Filter fast path ([`m6-filter-fast-path.md`](m6-filter-fast-path.md)) | **Gated.** Built only if, after tasks 3–5, the filter pass is the longest stage left. It is a case-insensitive substring search over the arena, with hits mapped back to keys |
| 7 | Close out M6 ([`m6-close-out.md`](m6-close-out.md)) | The docs and known limits are updated, the version is bumped, and the release is cut after confirmation |

## Order and gating

1. **Task 1 first.** Every later task proves itself against its numbers.
2. **Task 2 next.** It is small and independent, and it fixes the most common case: collapsing a
   group at scale. Task 3's fold stage reuses it.
3. **Task 3 is the user-visible guarantee.** After it, no frame waits on a rebuild at any size.
4. **Tasks 4 and 5 in order.** Task 5 consumes task 4's LCP output.
5. **Task 6 is decided after task 5,** using the new task 1 measurements.
6. **Task 7 last.**

## How each task runs

The same process as M5:

- Each task starts from synced `main` on its own `m6-<name>` branch.
- One Sonnet subagent works through the task's plan doc phase by phase.
- Each task finishes with:
  - fmt, clippy `-D warnings`, `cargo test --workspace` and the core boundary check;
  - **the perf suite in release, one test at a time, before and after, with the numbers in the
    PR**;
  - the Docker suite (foreground and bounded);
  - a PR that is opened but not merged, and an `## Outcome` section in the task's plan doc.

## CLAUDE.md rules M6 binds throughout

- **The core owns no clock.** Slices are a count of keys, not a duration, calibrated by the perf
  suite.
- **The render loop never does I/O, and a keystroke is answered in one frame.** This is the
  rule M6 exists to make true at 1M keys.
- **The key list is columnar and capped.** New per-key data is a parallel array (`Vec<T>`, no
  per-key struct), reported in `heap_bytes`, cleared in `LoadedSet::clear`.
- **The cap is enforced in exactly one place** (`scan_batch`), and that is unchanged.
- **Filter keystrokes narrow or defer, and never rescan per keystroke.** Narrowing stays
  synchronous. Deferred rebuilds become jobs.
- **Degradation is always visible.** A stale list while a job runs is labelled.
