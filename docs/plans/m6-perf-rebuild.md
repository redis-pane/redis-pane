# M6: Rebuilds at real SCAN order

Status: **planned, not started.** Its own milestone, after the beta (PRD §9), so that M4 can
close out on what it has. Found while planning an M4 follow-up on 2026-10-04 and deferred by the
user's decision to focus on `0.1.0-beta.1`.

## Context

M4 task 4 left three paths above the 16 ms frame budget at 1M keys, measured by the perf harness:
tree toggle ~44 ms, the worst scan page ~32 ms, and the debounced full filter rebuild ~22 ms.

**Those numbers are optimistic by roughly 10×.** `crates/core/tests/perf.rs` pushes keys in
already-sorted order (`user:00000000:session`, `user:00000001:session`, …). Real `SCAN` returns
keys in hash-slot order, which is effectively random. That defeats two things the harness
silently relied on:

- **the sort**: Rust's pattern-defeating quicksort is near-linear on already-sorted input, and
  every comparison goes through `offsets[i]` into the arena, so in random order each one is two
  cache misses;
- **the tree fold's locality**: `Tree::rebuild` walks names in name order, which in a
  scan-ordered arena is a random walk through 40 MB.

Measured on 2026-10-04 (Apple Silicon, release, median of 7), with a throwaway profiler over the
real `KeyView::rebuild`, `Tree::rebuild` and `State::rebuild_list`. Deep names are
`app:{i%7}:tenant:{i%211}:user:{i:08}:session`, a realistic multi-level keyspace:

| At 1M keys | Harness order (pre-sorted) | Random order, flat names | Random order, deep names |
|---|---|---|---|
| View rebuild, scan order, no filter | 1.3 ms | 1.3 ms | 1.3 ms |
| View rebuild, scan order, filter `user:0000` | 23 ms | 24 ms | 41 ms |
| View rebuild, name sort | 4 ms | **240 ms** | **276 ms** |
| `Tree::rebuild` alone (over a name-sorted view) | 38 ms | **139 ms** | **205 ms** |
| `rebuild_list` in tree mode (= tree toggle) | 42 ms | **373 ms** | **482 ms** |

The worst scan page in tree mode is one scheduled full rebuild near the end of the scan, so at
real order it is estimated at ~300–400 ms, not 32 ms (not yet measured: phase 0 measures it).

**Constant-factor tuning cannot close this gap.** Sorting `(u128 name-prefix, index)` pairs, with
a full compare only on ties, cut the flat-name sort from 240 ms to 41 ms, but deep names share long
prefixes and it only reached 165 ms. Nothing that still sorts 1M keys per keystroke gets to 16 ms.

## Goal

At 1M keys in real SCAN order, no keystroke and no scan page costs more than one 16 ms frame, and
the work a rebuild does in total is reduced so that a rebuilt list appears quickly. Specifically:

1. **Every frame stays within budget**, however large the keyspace, because no single `update`
   does unbounded work.
2. **Total rebuild work drops**, so the time until the new list appears is short (target: under
   250 ms at 1M random-order keys, deep names).
3. **The harness measures the real case**, so these budgets mean something.

## Decisions

1. **First, make the harness honest, before changing any code.** The perf fixtures
   gain a SCAN-like order (a fixed-seed shuffle) and a deep-name keyspace alongside today's.
   Re-baseline every budget against them, including the whole-scan worst page, and record the
   numbers in this doc and in `m4-perf-harness.md`. Ceilings that the new fixtures break are
   re-set from the new measurements (1.5× the CI figure, M4 task 2's rule) so CI stays green
   without hiding the regression from view. Same principle as M4 task 2: measure first, so the
   wins are measured, not claimed.

2. **A persistent name order, maintained as keys arrive, replaces sorting at rebuild time.** The
   Loaded set gains a name-ordered permutation of all loaded indices (`Vec<u32>`, 4 MB at 1M),
   kept up to date incrementally: each scan page is sorted on its own (500 keys) and merged in.
   To keep merges from being O(n) per page, they follow the same geometric schedule as M4 task 3's
   rebuilds — a small unmerged tail is merged when it reaches a fraction of the whole — and a
   lookup that needs the full order merges the tail first. With that in place:
   - a Name-sort view is a filtered walk of the permutation, O(n) and sequential, with no sort;
   - tree mode is the same walk plus the fold;
   - a filter rebuild in Name/tree mode walks the permutation instead of sorting what matched.

   **Confirm at build time:** whether a cached sort-key prefix (measured above) is still worth
   adding to make the merges cheaper, and the merge threshold. Memory cost must be reported
   against the 250 MB budget (today 83.4 MB at 1M).

3. **Rebuilds become a resumable job, done in bounded slices across frames.** This is what
   guarantees the Goal's first point independent of n. A rebuild that would touch more than a
   fixed number of keys (the slice size) no longer runs inside one `update`:
   - the core records a `RebuildJob` (what is being built, a cursor, the partial result) and
     returns a command asking the shell to continue it;
   - the shell sends `Msg::RebuildStep` back (after drawing a frame, so input and drawing are
     never starved); each step processes one slice;
   - the **old** list stays on screen and stays navigable until the job finishes, then the new
     one replaces it in a single swap, with selection and the Open key relocated as today;
   - a newer trigger (another keystroke, a sort change, a scan page that needs a rebuild)
     replaces the running job rather than queueing behind it;
   - the header shows that the list is being rebuilt, with progress, so a stale list is never
     silent (the same principle as ADR-0006: degradation is always visible).

   The slice size is a **count of keys, not a duration** — the core owns no clock (CLAUDE.md).
   It is calibrated by the perf suite so that one slice fits comfortably inside 16 ms on CI.

   **Confirm at build time:** how the slicing applies to each stage (the filter pass and the tree
   fold slice naturally; a merge slices by output position). Whether small rebuilds (under one
   slice) stay synchronous, so nothing changes for small keyspaces — **recommended: yes**, so
   every existing test and golden that rebuilds a small list is unaffected.

4. **The filter pass gets faster where it is cheap to.** A glob with no wildcards (the common
   case: a substring) can search the arena as one contiguous buffer and map each hit back to its
   key by binary search over `offsets`, instead of testing a million keys one by one.
   **Confirm at build time** whether it is worth it once decision 3 bounds the frame cost anyway;
   it shortens how long the "rebuilding" state lasts, nothing more.

5. **Equivalence is the correctness proof, as in M4 tasks 3 and 4.** Every new path must produce the
   identical `KeyView`, `Tree` and selection that today's synchronous `rebuild_list` produces,
   for every sort, tree mode, filter and mid-scan change, including a job interrupted by a newer
   trigger and a scan page arriving while a job runs.

## Out of scope

- Changing what a sort or a filter means. Same results, computed differently.
- Lazily-fetched sorts (TTL, Size, Kind). They sort only what has arrived and stay as they are;
  decision 3's slicing applies to them for free.
- Cluster (ADR-0021).

## Files likely touched

| File | Change |
|---|---|
| `crates/core/tests/perf.rs` | SCAN-order and deep-name fixtures; re-baselined budgets; new budgets for job completion |
| `crates/core/src/state/loaded.rs` | the persistent name order and its incremental merge |
| `crates/core/src/state/view.rs` | Name sort as a walk of the name order; filter pass changes |
| `crates/core/src/state/tree.rs` | fold driven by the name order; resumable |
| `crates/core/src/state/mod.rs` | `RebuildJob`; `rebuild_list` starts or runs a job |
| `crates/core/src/msg.rs`, `command.rs` | `Msg::RebuildStep`, the continue command |
| `crates/core/src/update/` | step handling; scan pages and keystrokes during a job |
| `crates/core/src/render/` | the "rebuilding" readout in the header |
| `crates/app/src/terminal.rs` | send `RebuildStep` after a draw |
| `docs/DESIGN.md` | the rebuilding readout |

## Testing

- **Equivalence** (decision 5), across random-order keyspaces, not only sorted ones.
- **Perf, release, one test at a time:** per-step cost at 1M random-order deep names within
  16 ms; time to job completion for tree toggle, sort change and filter rebuild; whole-scan worst
  page within 16 ms; memory within 250 MB.
- **Golden frames:** the rebuilding readout; every existing golden unchanged (small lists stay
  synchronous).
- **By hand:** `fixtures.py -n 1000000` (whose keys arrive in real SCAN order) with tree mode
  toggled, sort changed and the filter edited while the scan is still running.
