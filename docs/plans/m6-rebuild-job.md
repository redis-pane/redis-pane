# M6 task 3: Sliced rebuild job

Status: **done.** This task delivers M6's main promise: no frame waits on a rebuild, at any
size.

## Context

[`m6-planning.md`](m6-planning.md) row 3. User decisions (2026-10-07): never freeze first, and
the old list stays on screen and usable while a rebuild runs. The core is pure and owns no clock
(CLAUDE.md), so a rebuild cannot run on a thread inside the core or stop itself after so many
milliseconds. It has to be a resumable piece of state that the shell steps forward one bounded
slice at a time. The shell already has the right pattern: `Command::ScheduleFilterRebuild` →
`Msg::FilterRebuildDue` (M4 task 4), handled in `crates/app/src/terminal.rs`'s event loop,
alongside the dirty flag and the 16 ms `FRAME`.

## Decisions

1. **`RebuildJob` in the core.** It holds:
   - what it is building: filter, mode, sort, tree mode, and a snapshot of the collapsed set;
   - its stage: `Filter` (offering keys in index order, producing the matched indices), `Sort`
     (the ordering for the view's sort), `Fold` (tree mode only, the `Tree::rebuild` loop state
     made resumable), then `Swap`;
   - a cursor within the stage;
   - the partial result in its own buffers, separate from `self.list`/`self.tree`, which keep
     showing the old list.

   Every stage advances by at most `SLICE` keys per step.
2. **Sorting in slices.** A comparator sort cannot pause partway. The `Sort` stage instead:
   - sorts fixed-size chunks (`SLICE` keys each, one chunk per step);
   - then merges them bottom-up, with each step merging at most `SLICE` output positions.

   Task 4 replaces the comparison with the fast record sort, inside the same stage shape. The lazy
   sorts (TTL, Size, Kind) use the same chunk-and-merge comparator.
3. **`SLICE` is a count of keys, not a time.** It is calibrated by the perf suite so that the
   slowest step on CI (a fold step at random order on deep names) stays comfortably within 16 ms.
   The calibration goes in the Outcome. It is one constant, `REBUILD_SLICE`.
4. **When a job is used and when it isn't.** A rebuild over fewer than `REBUILD_SLICE` keys
   stays synchronous: it runs today's code path and finishes inside one `update`. So every
   existing unit test and golden frame, all of which build small lists, is unchanged.
   - The **large** paths that start a job:
     - `filter_rebuild_due`;
     - `cycle_sort`;
     - `toggle_tree`;
     - collapse and expand, which become a fold-only job via task 2's `refold`;
     - `scan_batch`'s scheduled rebuild.
   - Every other caller of `rebuild_list` keeps calling it synchronously; they are
     correctness-critical and rare. The Outcome lists each call site and the path it takes.
5. **Replace or coalesce:**
   - **User triggers replace a running job.** Typing, sort, toggle, collapse, `Esc` on the
     filter: the old job is dropped, and a new one starts from the current settings.
   - **A scan page never replaces a running job.** It marks the job dirty. When the job swaps,
     the new keys are folded in at once: either they fit in one slice and run synchronously, or
     the next job starts. This prevents starvation late in a scan, where a job outlasts the gap
     between pages.
   - **A rescan or a cap abort the job.** `scan_started` clears the Loaded set, so every index
     is renumbered.
   - **Narrowing stays synchronous and is unchanged.** It cancels a running job, because the
     narrowed list is current.
6. **The shell's side:**
   - A running job emits `Command::ContinueRebuild` from `update`.
   - The shell answers with `Msg::RebuildStep`, but only after the next draw and after any input
     that is already waiting. Input and drawing are never starved. Between steps the shell's
     channel is the only scheduler.
   - The step carries no data; the job lives in the core state.
   - A `RebuildStep` that arrives with no job running, for example after a replace, is ignored.
7. **The swap**, in one `update`:
   - The finished buffers replace `self.list` and `self.tree`, and the new `Applied` record is
     set.
   - The selection is relocated by key index and the Open key relocated, with the same code
     `rebuild_list` uses.
   - `scan_last_rebuild_len` is set to the job's coverage.
   - The selection is saved when the job starts: if the user moved on the old list, the move is
     respected by key index.
8. **What the user sees during a job:**
   - The old list stays fully navigable. `Enter` opens a key; keys and rows refer to indices,
     which stay valid until a rescan, and a rescan aborts the job.
   - The header shows `rebuilding N%`, where N is the stage-weighted progress. It goes through
     the theme and glyphs and has golden frames.
   - Filter text typed while a job runs shows at once. The rows catch up at the swap, the same
     way a deferred rebuild's rows catch up today.
9. **Memory.** During a job the new buffers sit alongside the old ones: order and inverse
   (~8 MB at 1M) plus rows and inverse (~25 MB). Report the peak against the 250 MB budget.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/{mod,view,tree}.rs` | `RebuildJob`; resumable filter, sort, fold; swap |
| `crates/core/src/update/{keys,scan,mod}.rs` | start, replace, coalesce or abort; `RebuildStep` handling |
| `crates/core/src/{msg,command}.rs` | `Msg::RebuildStep`, `Command::ContinueRebuild` |
| `crates/core/src/render/mod.rs` | the `rebuilding N%` readout |
| `crates/app/src/terminal.rs` | step after draw, behind input |
| `crates/core/tests/perf.rs` | per-step cost and time until the new list appears |
| `docs/DESIGN.md` | the readout |

## Testing

- **Equivalence (the correctness proof, property-style).** For random keyspaces in random order
  with random filters, sorts, tree mode and collapsed sets, running a job to completion step by
  step produces exactly what `rebuild_list` produces. That covers `KeyView`, `Tree`, both
  inverses, the selection and the Open-key row. The same must hold:
  - with scan pages arriving between steps (coalesced: the next job or the synchronous tail
    brings it level);
  - with a job replaced by a newer trigger mid-stage;
  - with a collapse during a job;
  - with a rescan mid-job (aborted, nothing swapped).
- **Perf** (task 1's harness):
  - **Per step:** the worst single `update` at 1M `random_deep` is ≤ 16 ms on CI, for every
    trigger and for the whole-scan run in tree mode. This is the task's acceptance figure.
  - **Time until the new list appears:** report it, and expect it to be about today's total
    plus the cost of slicing, because tasks 4 and 5 shorten it.
- **Golden frames:** the `rebuilding N%` readout at 140 and 80 columns, plus ASCII. No existing
  golden moves.
- **Shell:** a `RebuildStep` is sent only after a draw, and pending input is handled first.
  Unit-test the scheduling logic if the event loop is too hard to test directly, and say so.
- **By hand:** `fixtures.py -n 1000000`. During the scan and on `t`, `s` and collapse, no
  freeze, and `rebuilding N%` appears and clears.

## Out of scope

Making each stage faster, which is tasks 4–6.

## Build design

Written before the code; decisions 1-9 above are unchanged. This section pins the shapes.

### Constant and threshold

`REBUILD_SLICE: usize` (keys; calibrated in phase 5, starting at 32,768) lives in
`state/rebuild.rs`. `State::rebuild_slice: Option<usize>` overrides it (a test and calibration
hook; `None` in the product). A rebuild is *large* when `keys.len() >= slice` (for a fold-only
job, when `list.len() >= slice`). Anything smaller runs today's synchronous code, unchanged.

### `RebuildJob` (`state/rebuild.rs`)

One struct, `Clone + Debug + PartialEq + Eq` (it lives in `State`), holding the target, the
stage, and the partial result in buffers that are not `self.list` / `self.tree`:

| Part | Fields |
|---|---|
| Target (snapshot at start) | `filter`, `mode`, `sort`, `want_tree: bool`, `covered = keys.len()`, `slice` |
| Kind | `Full` (filter, sort, inverse, then fold if `want_tree`) or `FoldOnly` (fold over the live, current `self.list`) |
| Stage | `Stage::{Filter, Sort, Inverse, Fold}` plus `dirty: bool` (scan pages arrived) |
| Filter stage | `cursor: usize` (next Loaded index), `order: Vec<u32>` (matches so far), `known: usize` (lazy sorts: matched rows with the sort column) |
| Sort stage | `scratch: Vec<u32>`, `SortPhase::{Chunks{next}, Merge{width, out, lo, mid, hi, i, j}}`; `order` is the merge source and `scratch` the destination, swapped after each pass |
| Inverse stage | `inverse: Vec<u32>`, `cursor` |
| Fold stage | `tree: Tree` (a shell: the collapsed set cloned, `separator`, own `rows` and `inverse`) and `FoldProgress` |

`FoldProgress` is `Tree::rebuild`'s loop state made a value: `next` view row, `previous` and
`segments` (the two swapped segment buffers), `open_groups`, `counts`, and a `Fixup{cursor}`
phase for the final pass that writes descendant counts and the inverse. `Tree::rebuild` itself is
now `fold_begin` + `fold_step(usize::MAX)`, so the synchronous and the sliced fold are one
code path and cannot drift.

The job's own buffers are what make "old list stays usable" true: `self.list`, `self.tree` and
`self.tree_mode` keep describing the *shown* list until the swap. `State::target_tree_mode()` is
`job.want_tree` while a job runs, else `tree_mode`; `toggle_tree` flips that, not the shown mode.

### Filter stage

Index range slicing: step `k` offers Loaded indices `cursor..min(cursor + slice, covered)` to the
filter, in index order, pushing matches to `order`. An empty filter pushes every index. For a lazy
sort it also counts rows that have the sort column (`known`), which is exactly what
`apply_sort` counts after filtering. A `Scan` sort skips the Sort stage: the filter order is the
answer.

### Sort stage (Name and the lazy sorts)

Same comparator as `KeyView::apply_sort`, as one function `order_cmp(keys, sort, a, b)`: Name is
the byte comparison of the names; Ttl, Size and Kind order what is known by value then by index,
and *unknowns go last in scan order* (index order), the rule of `sort_by_lazy`. The sort is
stable (chunks use `sort_by`, merges take the left run on ties), so the result is identical to
the single stable `sort_by`.

- **Chunk steps:** one step sorts one chunk of `slice` positions of `order` in place.
- **Merge steps:** bottom-up passes with run width `slice`, `2*slice`, ... until one run covers
  `order`. A step merges runs pairwise from `scratch`'s point of view, emitting at most `slice`
  output positions into `scratch` and remembering `(lo, mid, hi, i, j, out)` so the next step
  resumes mid-pair. When a pass has written every position, `order` and `scratch` swap and the
  width doubles. After the last pass `scratch` is dropped.

Task 4 replaces `order_cmp` and the chunk sort with the record sort inside the same shape.

### Inverse stage

`inverse = vec![NO_ROW; covered]` then `inverse[order[r]] = r`, `slice` rows per step. It is a
stage of its own because a 1M-row scatter is several ms.

### Fold stage (tree mode)

`Tree::fold_step` processes `slice` view rows (the segment compare, group rows, counts), then
`slice` rows of the fixup pass. `FoldOnly` jobs reach it directly, reading the live
`self.list`'s order; `Full` jobs read the job's own `order`. The job's tree shell is built from
the live tree's *current* collapsed set at job start, so a collapse during a job replaces the job
(decision 5) and the new job sees the toggle.

### The swap, in one `update`

1. `selected_index = key_at(view.selected)` on the *old* structures, captured **at the swap**,
   not at job start: that is what makes a move made on the old list respected, and it is the same
   instant `rebuild_list_with` captures it.
2. `list.install(order, inverse, known, Applied{filter, mode, sort, covered})` (Full jobs).
   `known` is the filter stage's count, `Applied` is built from the job's snapshot, **not** the
   typed text, so `can_narrow` / `is_current_for` keep their meaning if the user typed during the
   job.
3. `self.tree = job.tree` and `self.tree_mode = job.want_tree` when a tree was built; for a
   non-tree target `tree_mode = false` and `tree` is left as it is (as `rebuild_list` does).
4. `scan_last_rebuild_len = covered` (Full jobs only; `refold` does not touch it).
5. `relocate_after_rows_changed(selected_index)`, then `after_move` in the caller (scroll, fetch
   metadata for the new visible rows).
6. If scan pages arrived (`dirty`, `keys.len() > covered`): a flat `Scan`-order view is brought
   level by `extend` (`O(new keys)`); otherwise, if `grown_enough(keys.len(), covered)`, the
   follow-up job starts at once. Else the next page's geometric schedule decides, exactly as
   after any rebuild. *Deviation from decision 5's wording:* the geometric gate stops a chain of
   back-to-back follow-up jobs from running for the whole of a scan.

### Progress

`progress() -> u8` is a stage-weighted fraction: Filter 1, Sort 4, Fold 4, Inverse 1 (only the
stages the job has), each stage's fraction being its cursor over its size (Sort: pass number and
output position over `1 + passes`). Monotonic, 0..=99 until the swap.

### Call sites of `rebuild_list` and the path each takes

| Call site | Path |
|---|---|
| `keys::filter_edited` narrowing (`rebuild_list_narrowing`) | synchronous; cancels a running job |
| `keys::filter_rebuild_due` | `rebuild_list_async` (job when large; replaces) |
| `keys::filter_key_keys_pane` Esc, and Enter with a rebuild owed | `rebuild_list_async` |
| `keys::cycle_sort` | `rebuild_list_async` |
| `keys::toggle_tree` | enter tree: fold-only job if `fold_is_current`, else full job; leave tree: synchronous `leave_tree_mode` when current, else full job |
| `keys::collapse_group`, expand in `open_selected` | `refold_async`: fold-only job if `fold_is_current`, else full job (replaces) |
| `scan::scan_batch` scheduled rebuild | `rebuild_list_coalescing`: starts a job when none runs; while one runs it only marks `dirty` |
| `scan::scan_batch` page while a job runs (any view) | marks `dirty`; neither `extend` nor a rebuild runs |
| `scan::scan_batch` cap | `rebuild_list_async` (aborts and restarts: indices stay valid, but the new job covers every key) |
| `scan::scan_batch` restored-selection rebuild | synchronous (once per session start; cancels a job) |
| `scan::scan_started` | synchronous on an empty set; aborts the job |
| `scan_complete`, `scan_cancelled`, `scan_failed`, `scan_interrupted` | `rebuild_list_async` (so completing a 1M tree-mode scan does not freeze) |
| `State::apply_session` | synchronous (before the first frame) |
| `State::refold` / `leave_tree_mode` fallbacks | synchronous (kept for the small path and tests) |

Everything else in the tree (viewer, editor, confirm, link, slowlog, dashboard) calls
`rebuild_list` only from `#[cfg(test)]` code.

### Shell

`update` appends `Command::ContinueRebuild` to the commands whenever a job is running after the
message (so it is true for every message, not only the one that started it). The shell keeps a
`RebuildGate { pending, drawn }`: `ContinueRebuild` sets `pending` (and clears `drawn` when it
was not already pending); a draw sets `drawn`; a message whose commands lack `ContinueRebuild`
clears `pending`. The select loop gets a last, lowest-priority arm
`yield_now(), if gate.ready()` producing `Msg::RebuildStep`, so input, replies and timers are
polled first, and a step is never sent before the frame that shows `rebuilding N%` has been
drawn. Steps run back to back between draws (a draw still happens at most once per `FRAME`).

## Outcome

Done in `crates/core/src/state/{rebuild,tree,view,mod}.rs`, `update/{keys,scan,mod}.rs`,
`render/mod.rs`, `crates/app/src/terminal.rs` and `crates/core/tests/perf.rs`. Decisions 1-9 hold,
with the deviations listed below.

### What was built

- **`RebuildJob`** (`state/rebuild.rs`): stages `Filter` -> `Sort` -> `Inverse` -> `Fold`, each
  advancing at most `REBUILD_SLICE` keys or output positions per step, into buffers separate from
  `self.list` / `self.tree`. `FoldOnly` jobs start at `Fold` over the live view. The sort stage is a
  stable chunk sort plus bottom-up merge with `order_cmp` (same semantics as `apply_sort`). The fold is
  `Tree::rebuild`'s loop made a value (`FoldProgress`); `Tree::rebuild` itself now runs the same
  `fold_step`, so the synchronous and the sliced fold are one code path.
- **`State`** gains `job` and `rebuild_slice` (a test and calibration hook), and
  `rebuild_list_async` / `rebuild_list_coalescing` / `refold_async` / `set_tree_mode_async` /
  `rebuild_step` / `run_rebuild_to_completion` / `target_tree_mode`. `State::tree_mode` stays the
  *shown* mode until the swap; the job's target is `target_tree_mode()`.
- **Shell:** `Msg::RebuildStep`, `Command::ContinueRebuild` (appended by `update` whenever a job is
  running), `RebuildGate` and a last, lowest-priority `select!` arm in `terminal.rs`.
- **Readout:** `rebuilding N%` in the status bar, Warn style, in place of `sorted by ...` while it runs.

### Per-call-site table

| Call site | Path |
|---|---|
| `filter_edited` narrowing | synchronous; cancels a running job |
| `filter_rebuild_due`; filter `Esc`; filter `Enter` with a rebuild owed | `rebuild_list_async` (job when `keys.len() >= slice`; replaces) |
| `cycle_sort` | `rebuild_list_async` (no-op while the *target* is tree mode) |
| `toggle_tree` | `set_tree_mode_async`: fold-only job when the view is current, full job otherwise; leaving tree mode is the synchronous `leave_tree_mode` when current, else a full job |
| `collapse_group`, expand in `open_selected` | `refold_async` (fold-only job when `fold_is_current`, else full job; replaces) |
| `scan_batch` while a job runs | marks it dirty; no `extend`, no rebuild |
| `scan_batch` scheduled rebuild (no job) | `rebuild_list_async` |
| `scan_batch` cap | `rebuild_list_async` (replaces the job so the new one covers every key) |
| `scan_batch` restored selection | synchronous (once per session start; cancels any job) |
| `scan_started` | synchronous on the emptied set; aborts the job (epoch bumped) |
| `scan_complete` / `cancelled` / `failed` / `interrupted` | `rebuild_list_async` (so completing a 1M-key scan in tree mode does not freeze) |
| `apply_session` | synchronous (before the first frame) |
| `refold` / `leave_tree_mode` fallbacks | synchronous; cancel any job |

### Deviations

1. **Scan-end rebuilds are async** (spec decision 4 said every other caller stays synchronous).
   A 1M-key scan ending in tree mode would otherwise freeze for ~0.5 s once; this is the
   one place the whole-scan acceptance figure needed it.
2. **The follow-up job after a dirty swap is gated by `grown_enough`** (the existing 25%
   geometric schedule) rather than unconditional. A flat scan-order view is brought level by
   `extend` at the swap. Otherwise an unconditional chain of jobs would run for the whole scan.
   Scan end always levels the list.
3. **The cap replaces the job** (a fresh one over every key) rather than only aborting it, so
   the list is whole when the scan stops.
4. **`rebuilding N%` is in the status bar**, not the title bar, beside the readouts it replaces
   (`sorted by ...`, which would describe an order not on screen). The title bar is full
   (DESIGN §2 priority).
5. **Selection at the swap** is captured at the swap (so a move made on the old list is respected,
   as decision 7 intends), not at job start. When the swap flips tree mode with the cursor still on
   row 0 (what `t` leaves), it is read exactly as the synchronous toggle reads it, so `t` lands
   the cursor where it always did.
6. **Collapse/expand during a job read the collapsed set, not the stale row flag**, so a second
   press while the first is still rebuilding is not an un-collapse (`group_expanded`).
7. **`Esc` does not cancel a running job.** The decisions do not give it that meaning (the
   typed settings are already applied); the old list is fully usable meanwhile. Flagged for
   task 7's review.
8. **Session snapshot saves the target tree mode.**
9. **Tree rows are not pre-reserved** (a `reserve(keys)` makes `Vec` doubling land on 64 MB
   instead of 33 MB at 1M keys; found by the memory test).

### Slice calibration (`REBUILD_SLICE = 32,768`)

`rebuild_slice_calibration_random_deep` (tree toggle from scan order at 1M `random_deep`, the heaviest
job, median of 3 runs; worst step per stage; ms; local / CI):

| Slice | Steps | Fold | Sort chunk | Sort merge | Filter | Inverse | Trigger to swap |
|---|---|---|---|---|---|---|---|
| 16,384 | 743 | 4.3-8.2 / 5.1 | 2.0-2.3 / 3.5 | 1.2-1.4 / 1.9 | 0.02 | 0.1 / 0.4 | 530-550 / 778 |
| 24,576 | 492 | 5.6-6.0 / 8.1 | 2.7-3.1 / 5.5 | 1.9-2.1 / 2.8 | 0.02 | 0.1 / 0.5 | 527-554 / 824 |
| **32,768** | **341** | **7.4-8.5 / 9.0** | **3.9-4.2 / 7.7** | **2.2-2.5 / 3.6** | **0.03** | **0.1 / 0.4** | **503-550 / 784** |
| 49,152 | 230 | 11.6-12.5 / 13.8 | 5.8-6.0 / 12.4 | 3.3-3.8 / 5.9 | 0.04 | 0.1 / 0.5 | 522-547 / 838 |
| 65,536 | 159 | 15.2 / 15.2 | 8.2-8.9 / 16.9 | 4.2-5.0 / 6.5 | 0.06 | 0.2 / 0.5 | 496-512 / 719 |

The fold step is the slowest at every slice locally (~0.25 ms per 1,000 keys); on CI the sort chunk
catches it from 49k up. 32,768 puts the slowest step at 7.4-8.5 ms locally and 9.0 ms on CI, with
about 1.8x headroom to 16 ms on CI (task 1 saw CI up to 1.9x slower on a bad run). Filter and inverse
steps are 100x cheaper than the slice implies; they could take a larger share, but they are a few
dozen steps in total and are left uniform for simplicity.

### Perf: before and after (ms, 1M keys, `random_deep`; local / CI)

"Before" is the single synchronous `update` (tasks 1-2 figures); "after" is the worst single
`update` (the trigger or one step) and, beside it, trigger to swap.

| Trigger | Before: worst update | After: worst update | After: trigger to swap |
|---|---|---|---|
| sort change (scan -> name) | 278 / 225 | 3.9-4.0 / 7.8 | 311-323 / 558 |
| tree toggle from scan order | 501 / 386 | 8.4-9.0 / 8.2 | 541-569 / 765 |
| tree toggle, Name-sorted flat view | 224 / 211 | 8.0-8.4 / 8.5 | 208-223 / 208 |
| collapse one group | 196 / 198 | 7.1-8.0 / 8.6 | 149-195 / 193 |
| expand one group | 223 / 222 | 6.7-8.2 / 7.6 | 158-210 / 160 |
| debounced filter rebuild | 40 / 37 | 1.45-1.5 / 2.0 | 40.5 / 58 |
| whole scan in tree mode: worst update | 411 / 327 | 9.0-9.4 / 9.2 | scan end to level list 503-536 / 795 |
| whole scan: updates over 16 ms | 12 of 2000 | **0** / **0** | |
| whole scan: total | 1829 / 1285 | 2.3-2.5 s / 3.6 s | |

Sorted control: worst update 0.3 ms (sort), 1.7-3.5 ms (tree toggle), 0.8 ms (filter), whole-scan
worst 2.3 / 3.9 ms.

Trigger to swap is about today's total locally for the fold paths and 1.1-1.4x for sort (the merge
passes cost 5 extra passes over the order). On CI it is 1.5-2.5x for sort and toggle, where the
single `sort_by` had been unusually fast; tasks 4 and 5 shorten it. The perf ceilings are set per the
harness rule from this PR's CI run: every worst-update test is held at 16 ms (AT TARGET: the 1.5x
rule lands below it); the time-to-swap ceilings are 1.5x the slowest figure observed, rounded up
(collapse 290, filter 88, sort 812, toggle 1126); whole-scan total gate 6 s (sorted 10 s).

### Memory (decision 9)

Peak heap during the heaviest job (tree toggle from scan order): the shown list 61.5 MB plus the
job's buffers 51.4 MB (order, merge scratch, inverse, the new tree's rows and inverse, the fold's
counts) = **112.9 MB**, against the 250 MB budget; 97.3 MB after the swap. (`job_memory_peak_*`;
arithmetic over capacities, not RSS.)

### Tests

- `state::rebuild::tests`: 1,200 randomised cases x 3 triggers (filter, sort with partly-known
  metadata, tree on/off, collapse, redo) where the job run to completion equals `rebuild_list`
  (list, tree, selection, offset, Open-key row), the shown list is untouched until the swap, the
  selection moved mid-job is respected, and progress is monotonic and below 100.
- `update::rebuild_tests` (16): which triggers start, replace, coalesce and abort; the cap; scan
  end; a collapse during a job (and a second press); `t` twice; scan pages between steps ending
  where a synchronous rebuild does; a job replaced mid-stage; a collapse during a full job.
- Shell: `RebuildGate` is unit-tested (when a step may run). The `select!` loop itself is not
  unit-testable without a terminal and a runtime; its rule is the gate plus arm order.
- Goldens: `browser_rebuilding_{140,80,ascii}`; no existing golden moved.
- Counts: core lib 999, golden 258, app 94 (default run); perf 47 `--ignored`; Docker 153
  `--ignored`.

### For tasks 4 and 5

- **Sort stage (task 4):** replace `order_cmp` and the `sort_by` of one chunk (`sort_step`'s
  `Chunks` arm) and keep the shape: a step sorts at most `slice` positions of `order` in place, then
  merge passes move `order` <-> `scratch` at most `slice` output positions per step with
  `Merge{width,out,mid,hi,i,j}` resumable state. A prefix-record sort must still produce the
  identical stable order (ties by index) and the `known` count is computed in the *filter* stage.
  Today's merge is 5 passes at 1M; a 4-way merge or fewer, larger runs would cut time to swap.
- **Fold stage (task 5):** `Tree::fold_step(keys, order, inverse_len, progress, limit)` handles
  `limit` view rows, then `limit` fix-up rows. An LCP column would replace the per-key byte
  comparison `shared` in `fold_key`; carry the LCP in `order` order (a parallel `Vec<u16>` produced by
  the sort) and thread it through `FoldProgress`. The fold is the slowest step, so any speed-up there
  lets `REBUILD_SLICE` rise and the step count fall.
- `REBUILD_SLICE` is a per-key constant tuned to the slowest stage; if a stage gets faster,
  re-run `rebuild_slice_calibration_random_deep`.
