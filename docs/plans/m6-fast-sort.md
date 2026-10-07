# M6 task 4: Fast name sort

Status: **planned.**

## Context

[`m6-planning.md`](m6-planning.md), row 4. After task 3, no frame waits on a rebuild. But at
1M keys, deep names, random order, the Name sort still takes 240–276 ms of work, and every
rebuild in tree mode pays it. It is slow because each comparison reads two names from the arena
at random offsets: dependent cache misses
([`m6-perf-rebuild.md`](m6-perf-rebuild.md) § Why it is slow). The goal is to sort contiguous
records instead of following offsets.

## Decisions

1. **A sort-prefix column.** `LoadedSet` gets `prefix: Vec<u64>`:
   - one entry per key, holding the name's first 8 bytes big-endian, zero-padded;
   - filled in `push` (the name is already in cache there) and cleared in `clear`;
   - counted in `heap_bytes`: 8 MB at 1M.

   The ordering must stay plain byte order, exactly what `name().cmp()` gives today. A zero pad
   alone would tie `"ab"` with `"ab\0"`, so ties are always resolved by the refinement below,
   never by the prefix alone.
2. **A record sort with tie refinement.**
   - Build `(prefix, index)` records for the rows to sort: one sequential gather from the prefix
     column.
   - Sort the records. Radix or pdqsort on contiguous records; choose whichever measures faster.
   - For each run of equal prefixes, refine:
     - gather the next 8 bytes of each tied name into the records (independent loads, which the
       CPU overlaps);
     - re-sort the run, and repeat until the run is resolved or a name is exhausted;
     - shorter names sort first, matching byte order.
   - Emit the LCP (bytes shared with the previous key in order) as a by-product. Task 5 uses it.
3. **It slots into task 3's `Sort` stage.**
   - Chunks are sorted with the record sort.
   - Merges compare records first and fall back to the full name only on equal prefixes.
   - Synchronous small rebuilds use the same function, so there is one sort implementation.
4. **A persistent name order: go or no-go.** After decisions 1–3, measure the time until the new
   list appears for tree toggle and sort change at 1M `random_deep`.
   - If the sort is still more than half of the remaining time to the new list, design an
     incrementally maintained name order as a follow-up task. Pages are sorted and merged on the
     scan's geometric schedule, so toggle and sort change skip the sort entirely.
   - Otherwise record "not needed" with the numbers.

   Either way, the decision goes in the Outcome.
5. **The target** is a Name sort at 1M deep names in random order of ≤ 80 ms total work, local
   release. Report CI's figure too.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/loaded.rs` | the `prefix` column (push, clear, `heap_bytes`) |
| `crates/core/src/state/view.rs` (or a new `state/sort.rs`) | the record sort, tie refinement, LCP |
| `crates/core/src/state/mod.rs` | task 3's `Sort` stage uses it |
| `crates/core/tests/perf.rs` | the sort-alone measurement tightened |

## Testing

- **Equivalence against `slice::sort_by(name cmp)`.** Property-style over random byte strings,
  covering:
  - empty names, names that are prefixes of others, and embedded `\0`;
  - long shared prefixes (deep names), and identical first 8 or 16 bytes;
  - non-UTF-8 bytes.

  The order must be identical, and the LCP must equal the naive LCP.
- **Task 3's job equivalence tests pass unchanged.**
- **Perf:** sort alone, time until the new list appears for toggle and sort change, and memory
  (the +8 MB reported).

## Out of scope

The fold, which is task 5, and the persistent order itself unless decision 4 says go. In that
case it becomes its own task doc.
