# M6 task 3: Sliced rebuild job

Status: **planned.** This task delivers M6's main promise: no frame waits on a rebuild, at any
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
