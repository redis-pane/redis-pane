# M6 task 2: Fold-only rebuild

Status: **planned.**

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
