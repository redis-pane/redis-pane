# M6 task 4: Fast name sort

Status: **done** (PR #81).

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

## Outcome

Built as designed, with one structural change: the Name sort is one resumable state machine
(`NameSorter`, `crates/core/src/state/sort.rs`) that the synchronous path runs with an unbounded
slice and the rebuild job steps a slice at a time. There is one implementation.

**Design as built**
- `LoadedSet.prefix: Vec<u64>` (first 8 bytes big-endian, zero-padded), filled in `push`, reset in
  `clear`, counted in `heap_bytes`. The sort pairs it with the name length (`rem`, 0 to 8), so
  `"ab"` and `"ab\0"` never tie.
- Records are `(key: u64, tie: rem << 32 | index)`. Chunks of `slice` records are sorted and merged
  pairwise on records alone, with no arena access. Runs that tie (equal key, `rem == 8`) are
  refined level by level: the next 8 bytes of each tied name are gathered in batches of 32 (spans
  first, then bytes, so the arena loads overlap), the run is re-sorted, and the still-tied
  subruns go on a worklist. A run larger than a slice is gathered and sorted by the same chunk and
  merge machinery, so no step exceeds a slice.
- Buffers (`recs`, `scratch`, `lcp`) grow a slice per Fill step, so no single step pays for
  faulting in 20 MB of pages.
- The job's Name sort uses the sorter. Lazy sorts (Ttl, Size, Kind) keep their comparator and
  chunk/merge unchanged.
- Precondition: the order handed in is ascending by index (every caller builds it that way). Equal
  names then keep index order, which is what the old stable sort gave.

**Radix versus pdqsort** (1M `random_deep`, local, `sort::tests::phase_times`, noisy machine)

| | chunk sort, whole sort (sync) | whole sort (sliced job) |
|---|---|---|
| pdqsort only | 24-28 ms chunk, 98-110 ms total | 18 ms chunk, 112-127 ms total |
| LSD radix on varying bytes (kept) | 5-9 ms chunk, 79-99 ms total | 4-5 ms chunk, 83-89 ms total |

Radix runs only on runs of at least 1024 records with at most 3 varying bytes (key bytes plus
`rem`), and is skipped for already-sorted records; everything else uses pdqsort.

**k-way merge: not built.** Measured pairwise merge on records is 10 ms of an ~85 ms job sort
(5 passes, ~125 us per step). A k-way merge could save at most a few milliseconds; the time is in
the refinement gathers (about 60 ms), not the merge. Recorded as considered and rejected on
measurement.

**LCP decision: stored, cheap.** The refinement writes each pair's LCP as a by-product (one
sequential write per position, 4 MB at 1M). The job keeps it as `RebuildJob::name_lcp()`, a
`Vec<u32>` parallel to the final order: `lcp[p]` is the number of bytes name `p` shares with name
`p - 1`, and `lcp[0] = 0`. Task 5 reads it during the fold stage. The synchronous `KeyView` path
computes it and discards it.

**Numbers at 1M `random_deep`** (local = Apple Silicon on a loaded machine, load avg ~6, so noisy;
CI = `perf` job, `ubuntu-latest`, run 37635160874)

| | before local / CI | after local / CI |
|---|---|---|
| Name sort alone | 280-304 / 228 ms | 71 / 120 ms (target <= 80 local: met) |
| Name sort alone, random_flat | 241 / 205 ms | 32-55 / 62 ms |
| Name sort alone, sorted control | 4 / 3.9 ms | 10.7-13.5 / 13.4 ms (pdqsort's best case lost) |
| Time-to-swap, sort change | 311-368 / 558-615 ms | 88-109 / 145-154 ms |
| Time-to-swap, tree toggle from scan order | 541-579 / 765-847 ms | 295-352 / 363-383 ms |
| Worst sort step (any stage) | 3.9-7 ms | 1.1-2.6 ms local, 1.4 ms CI |
| Worst fold step (unchanged code, task 5's lever) | 8 ms local | 8-17 ms local (machine noise), 9.3 ms CI |
| Whole scan, tree mode, total | 2.4 s local, 1.3 s CI | 1.1-1.2 s local, 1.36 s CI |
| Memory: LoadedSet | 56 MB | 64 MB (+8 MB prefix) |
| Memory: shown list + job buffers peak | 112.9 MB | 124.4 MB |

**Calibration** (`rebuild_slice_calibration_random_deep`, local): after the change a sort or merge
step is under 3 ms at every slice from 8192 to 65536 (refine 0.3-2.7 ms, chunk 0.3-0.9 ms, merge
under 0.3 ms). The fold is the only stage over 16 ms (at 49152 and above). `REBUILD_SLICE` stays
32768; raising it is task 5's decision once the fold allows.

**Ceilings re-set** from CI (1.5x the slowest seen, rounded up): Name sort sorted 21, random_flat
94, random_deep 180 ms; time-to-swap sort change 231, tree toggle 575 ms. Per-step gates stay 16 ms.

**Persistent name order: no-go ("not needed").** Time-to-swap after decisions 1-3: sort change
88-109 ms local / 145 ms CI, tree toggle 295-352 ms local / 363-383 ms CI. For tree toggle the sort
is about 85 ms of ~300 (under 30%), and the rest is the fold, so a persistent order would not move
the number that matters. For a plain sort change the sort is nearly all of the time, but the total
is 88-145 ms, well inside the 250 ms goal. Re-check after task 5: if the fold drops to ~60 ms,
toggle is roughly half sort, and the question reopens with the then-current numbers.

**Deviations**
- The three `golden_rebuilding_readout_*` goldens changed in one line each (the `rebuilding N%`
  percentage), because the progress weights now follow the sorter's phases. Nothing else in any
  golden moved.
- No k-way merge (above).
- Lazy sorts do not share the new merge; they keep task 3's chunk/merge.
- The sorted-control name sort is slower than pdqsort's already-sorted fast path (4 ms to 13 ms);
  its ceiling is raised accordingly.
- Local fold-step figures varied 8-17 ms during this task because the machine was loaded (the
  baseline binary showed the same range at the same time); CI's fold step was 9.3 ms.
