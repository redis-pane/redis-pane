# M6 task 2: Fold-only rebuild

Status: **done.**

## Context

[`m6-planning.md`](m6-planning.md), row 2. In tree mode the list is a fold (`Tree::rebuild`)
over a view that is already filtered and sorted by name (`KeyView`). Two common actions change
only the fold, yet they still rebuild the whole list:

- **Collapsing or expanding a group.** `collapse_group` and the expand path in
  `crates/core/src/update/keys.rs` call `state.rebuild_list()`, which re-filters and re-sorts
  every loaded key before folding.
- **Entering tree mode (`t`) when the flat view is already in Name order.** Name is the only
  sort tree mode allows, so the view needs no re-sort; only the fold is new.

At 1M keys in random order, the sort is 240–276 ms of the 373–482 ms total
([`m6-perf-rebuild.md`](m6-perf-rebuild.md)).

## Decisions

1. **Add `State::refold()`.** It runs the second half of `rebuild_list_with`:
   - `Tree::rebuild` over the current `self.list`;
   - then the same selection relocation and `relocate_open_key` steps.

   It does not touch `self.list`, `filter_pending`, or `scan_last_rebuild_len`. Only the scan
   schedule reads `scan_last_rebuild_len`, and a refold adds no keys.
2. **When refold is valid:**
   - tree mode is on;
   - `self.list.sort == SortBy::Name`;
   - the view covers every loaded key (`KeyView`'s `Applied.covered == keys.len()`);
   - no filter rebuild is pending.

   If any of these is false, it falls back to `rebuild_list`. The check lives in one place, a
   `State::fold_is_current()` helper, so no caller decides it alone.
3. **Callers:**
   - collapse and expand use `refold()`;
   - `toggle_tree` into tree mode uses `refold()` when the flat view is already Name-sorted and
     covers every loaded key;
   - leaving tree mode needs no fold at all, because the flat view is the existing `self.list`.
     It still runs the selection relocation, through a shared helper.
4. **No change** to `Tree::rebuild` itself. Tasks 3 and 5 change it.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/mod.rs` | `refold`, `fold_is_current`, shared relocation helper |
| `crates/core/src/update/keys.rs` | collapse, expand and toggle use it |

## Testing

- **Equivalence (property-style, as in M4):** for random keyspaces in random order, random
  filters and random collapsed sets, `refold()` produces exactly the same `Tree` rows, inverse,
  selection and Open-key row as `rebuild_list()`. This includes a fallback case where the view
  does not cover the latest scan page.
- **Perf (task 1's harness):** at 1M keys with `random_deep`, collapse and expand go from
  sort-plus-fold to fold only. Report the measured gain.
- **Existing tests:** every existing tree and collapse test and golden passes unchanged.

## Out of scope

Slicing the fold (task 3) and making it faster (task 5).

## Outcome

Done as specified, in `crates/core/src/state/mod.rs`, `state/view.rs` and `update/keys.rs`.

- **`State::fold_is_current()`** is tree mode on, `list.sort == Name`, no `filter_pending`, and a
  new read-only `KeyView::is_current_for(keys.len())` (the `Applied` record covers every loaded
  key and was built for the typed filter, mode and sort; `Applied` stays private). The
  filter/mode/sort comparison goes beyond the spec's `covered == keys.len()`: a typed filter
  that differs from the applied one can only happen with a rebuild owed, but checking it makes
  the fast path not depend on `filter_pending` alone.
- **`State::refold()`** is `Tree::rebuild` plus the shared tail `relocate_after_rows_changed`
  (selection relocation then `relocate_open_key`), now also the tail of `rebuild_list_with`. It
  does not touch `list`, `filter_pending` or `scan_last_rebuild_len`, and falls back to
  `rebuild_list` when `fold_is_current()` is false.
- **Callers.** Collapse and expand call `refold`. `toggle_tree` into tree mode calls `refold`
  (it falls back unless the flat view is Name-sorted, current and has no rebuild owed).
  Out of tree mode it calls the new `State::leave_tree_mode`, which runs only the shared tail
  when the view is current and falls back otherwise.
- **Leaving tree mode, what today does.** Tree mode forces the flat sort to Name and
  `rebuild_list` never restores the user's earlier sort, so after `t` off the flat list stays
  Name-sorted. `rebuild_list` over a current Name view re-produces the same order, so skipping
  it is exact. Behaviour is preserved, including that sort; the equivalence test covers it.
- **Audit of other fold-only callers.** None. `Tree::toggle` is called only from collapse and
  expand. Session restore sets `tree_mode` but not the collapsed set, and runs before the view
  exists, so it needs the full rebuild it already does.

### Equivalence tests (`state::refold_tests`, splitmix64, fixed seeds)

Each compares `Tree` (rows, inverse and collapsed set), `KeyView` (order, inverse, `applied`,
`known`), tree mode, `filter_pending`, selection, scroll offset and the Open key's row against a
clone put through `rebuild_list()`: collapse/expand on 400 random keyspaces (3 toggles each,
random glob/fuzzy filters including wildcards, random collapsed sets, selection and Open key),
the fallback after a scan page outran the view (200), the `filter_pending` fallback (200), and
tree toggle in and out, with the flat sort Name or Scan (400). Every existing test and golden
passes unchanged.

### Measurements (ms, 1M keys, release)

| Measurement | Before local / CI | After local (2 runs) / CI |
|---|---|---|
| collapse, random_deep | 474 / 372 | 196, 205 / 198 |
| expand, random_deep | 504 / 390 | 226, 220 / 222 |
| collapse, random_flat | 381 / 263 | 140, 135 / 138 |
| expand, random_flat | 399 / 327 | 156, 152 / 166 |
| time-to-new-list, collapse (deep) | 482 / 360 | 191, 196 / 193 |
| toggle tree, scan-order flat view (deep) | 500 / 386 | 509 / 533 (unchanged: still sorts) |
| toggle tree, Name-sorted flat view (deep) | not measured | 224, 229 / 211 |
| toggle tree off (deep) | not measured | 0.07, 0.05 / 0.12 |

Collapse and expand now cost the fold alone (221 ms local fold-alone figure from task 1; the
collapse path is a little under it because the group is partly collapsed). The CI "before"
column is task 1's figures; the main-branch rerun here matched task 1 locally.

Ceilings were tightened to 1.5x the slowest figure seen (two local runs and one CI run; a second
CI sample was requested but the rerun did not produce a log): collapse 307 (flat 211), expand 340
(flat 249), time-to-new-list collapse 295, tree toggle from a Name-sorted view 344, tree toggle
off 10. Task 1 saw CI up to 1.9x slower on some runs, so these may need loosening once if CI
noise trips them.

### For task 3 (sliced rebuild)

- A refold is valid only while `fold_is_current()`; a sliced rebuild that has not yet swapped in
  its new view leaves `list.is_current_for(keys.len())` false, so `refold` falls back to
  `rebuild_list` by itself. Task 3 should decide whether a collapse during a slice cancels it or
  queues behind it.
- `relocate_after_rows_changed(selected_index)` is the single place that puts the selection and
  Open key back after rows move; capture `selected_index` (`key_at(view.selected)`) *before* the
  rows change and call it after the swap. `toggle_tree` resets `selected` to 0 before the
  rebuild, so the captured key there is whichever the old structures put on row 0.
- `scan_last_rebuild_len` is deliberately untouched by `refold`.
