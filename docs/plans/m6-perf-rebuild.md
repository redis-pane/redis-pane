# M6: Rebuilds at real SCAN order

Status: **planned, not started.** The tasks are listed in [`m6-planning.md`](m6-planning.md). M6
is its own milestone, after the beta (PRD §9). It was found on 2026-10-04 while planning an M4
follow-up, deferred so M4 and then M5 could ship, and analysed in depth on 2026-10-07.

## The problem

At a million keys arriving in real `SCAN` order, the key list freezes the screen for 0.4–0.5 s
whenever you:
- toggle the tree (`t`),
- change the sort (`s`),
- or let a debounced filter rebuild run.

Tree mode is the default, so the same full rebuild also runs about 30 times while a large scan is
in progress (decision 3 below). The last several of those each freeze the screen for 0.3–0.5 s.
Collapsing or expanding one group costs nearly as much, because it rebuilds everything.

The M4 perf harness missed all of this. It pushes `user:00000000:session`,
`user:00000001:session`, … already in name order. Real `SCAN` returns keys in hash-slot order,
which is effectively random.

## Measurements

Measured on 2026-10-04 on Apple Silicon, release build, median of 7 runs. A throwaway profiler
timed the real `KeyView::rebuild`, `Tree::rebuild` and `State::rebuild_list`. Deep names are
`app:{i%7}:tenant:{i%211}:user:{i:08}:session`.

| At 1M keys | Harness order (pre-sorted) | Random order, flat names | Random order, deep names |
|---|---|---|---|
| View rebuild, scan order, no filter | 1.3 ms | 1.3 ms | 1.3 ms |
| View rebuild, scan order, filter `user:0000` | 23 ms | 24 ms | 41 ms |
| View rebuild, name sort | 4 ms | **240 ms** | **276 ms** |
| `Tree::rebuild` alone (over a name-sorted view) | 38 ms | **139 ms** | **205 ms** |
| `rebuild_list` in tree mode (= tree toggle) | 42 ms | **373 ms** | **482 ms** |

The worst scan page in tree mode is a scheduled full rebuild late in the scan. It has not yet
been measured at real order; task 1 measures it.

## Why it is slow

**The cause is the access pattern, not the algorithm.**
- Names live in one arena in arrival (index) order, and reading it in index order is sequential
  and cheap. A full filter-free view rebuild, for example, takes 1.3 ms.
- A pass in name order touches that arena at random. Every comparison in the sort is two
  dependent cache misses through `offsets[i]`. The fold reads every name once, in name order.
  - The same fold over a pre-sorted arena takes 38 ms; at random order it takes 139–205 ms.
    About 100 ms of the difference is memory latency.
  - Pattern-defeating quicksort is near-linear on pre-sorted input, so the harness also hid the
    sort's real cost.

**Constant-factor tuning of the existing comparator sort does not close the gap.** Sorting
`(u128 name-prefix, index)` pairs, with a full comparison only on ties, cut the sort from 240 ms
to 41 ms on flat names. On deep names it only reached 165 ms, because the ties went back through
the arena one dependent comparison at a time. And no sort plus fold of 1M keys fits in 16 ms
anyway.

## Goals

All of these are measured at 1M keys, deep names, random order, by task 1's harness:

1. **No frame waits on a rebuild.** No single `update` exceeds 16 ms on CI, however large the
   keyspace.
2. **A rebuilt list appears quickly.** Tree toggle, sort change and filter rebuild show the new
   list within 250 ms in total.
3. **The harness measures the real case**, so these budgets mean something.
4. **Memory stays inside 250 MB.** Today it is 83.4 MB at 1M.

## Decisions

1. **Never freeze first** (user, 2026-10-07). Slicing rebuilds across frames is built before any
   algorithm work. It is the only thing that bounds a frame independently of n, and the speedups
   after it only shorten how long the rebuilding state lasts.
2. **While a rebuild runs, the old list stays on screen and stays usable** (user, 2026-10-07).
   - Navigation and opening a key work on the old list.
   - The header shows `rebuilding N%`, so a stale list is never silent, the same principle as
     ADR-0006.
   - When the job finishes, the new list replaces the old one in a single swap, with the
     selection and the Open key relocated as `rebuild_list` does today.
3. **The code facts that shape the build**, found 2026-10-07:
   - **The scan rebuilds the whole list in tree mode.** `scan_batch` appends cheaply only in
     flat Scan order. In tree mode, the default, `grown_enough` (25% growth) triggers a full
     synchronous `rebuild_list`, about 30 times on the way to 1M. The everyday case freezes
     during the scan, not only on `t`.
   - **Collapsing or expanding a group rebuilds everything.** `collapse_group` (`keys.rs`) calls
     `rebuild_list`, which re-filters and re-sorts 1M keys when only the fold changed.
   - **Many production callers rely on `rebuild_list` being synchronous**, across `keys.rs`,
     `scan.rs`, `session.rs`, `link.rs`, `confirm.rs`, `update/mod.rs` and `render/mod.rs`, so
     **synchronous stays the default**. Only the large paths start a job: user-triggered
     (filter rebuild, sort, tree toggle, collapse) and scan-triggered.
   - **Starvation:** late in a 1M scan, a job outlasts the gap between pages. A scan page
     therefore never replaces a running job. It marks the job dirty, and another job runs after
     the swap. Only user actions replace a job, and a rescan aborts it.
   - **Glob matching is case-insensitive** (`eq_ci` in `glob_at`), so a fast substring path over
     the arena must be too.
4. **Sort contiguous records, not arena offsets.**
   - A fixed-width sort prefix per key is stored at push time, which is sequential and cheap.
   - A name sort becomes a sort over contiguous `(prefix, index)` records.
   - Tied runs are refined by gathering the next chunk of each tied name. Those gathers are
     independent loads, which the CPU overlaps, unlike the dependent loads inside a comparator.
   - The sort also emits each key's LCP with its predecessor, which the fold can use to skip
     comparing segments it already knows are shared.
5. **A persistent name order is built only if task 4's measurements call for it.** Keeping a
   name order up to date as keys arrive would remove the sort from toggle and sort change
   entirely. It costs merges during the scan and more state to keep consistent, so it is not
   assumed.
6. **Equivalence is the correctness proof, as in M4 tasks 3 and 4.** Every new path produces
   exactly the `KeyView`, `Tree`, selection and Open-key row that today's synchronous
   `rebuild_list` produces. That covers every sort, tree mode, filter and collapsed set, keys
   arriving mid-scan, a job replaced by a newer trigger, and a scan page arriving while a job
   runs.

## Out of scope

- Changing what a sort or a filter means. The results stay the same; only how they are computed
  changes.
- Lazily fetched sorts (TTL, Size, Kind). They sort only what has arrived, and they get the
  slicing for free.
- Cluster, which shipped in M5. A merged scan is still random order, so everything here applies
  to it unchanged.
