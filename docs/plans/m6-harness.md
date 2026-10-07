# M6 task 1: Honest harness

Status: **done** (PR #78).

## Context

[`m6-planning.md`](m6-planning.md) row 1. `crates/core/tests/perf.rs` (M4 task 2) builds every
fixture with `key_name(i) = user:{i:08}:session`, pushed in index order. That order is already
the name order. Real `SCAN` order is effectively random, and the 2026-10-04 measurements in
[`m6-perf-rebuild.md`](m6-perf-rebuild.md) show the harness understates rebuild costs by 5–10×.
Every later M6 task is judged by these numbers, so they have to describe the real case first.

## Decisions

1. **Fixtures.** Every fixture stays deterministic, and no new crate dependency is added: the
   shuffle uses a small xorshift or splitmix generator written in the test file.

   | Fixture | Name shape | Push order | Role |
   |---|---|---|---|
   | `sorted` | today's | index order | control, so the old numbers stay comparable |
   | `random_flat` | today's | fixed-seed shuffle | the real order with today's names |
   | `random_deep` | `app:{i%7}:tenant:{i%211}:user:{i:08}:session` | fixed-seed shuffle | the reference fixture for M6's goals |

2. **Measurements**, at 1M keys on the fixtures named:
   - **Existing tests:** every test that exists today, run on `random_deep` as well as on
     `sorted`.
   - **New, on `random_deep`:**
     - collapsing one top-level group;
     - expanding it again;
     - the fold alone (`Tree::rebuild` over a name-sorted view);
     - the Name sort alone.
   - **The whole-scan worst page, in tree mode, on `random_deep`:** pages of 500 pushed through
     `update`, reporting the worst page and the total.
   - **Time until the new list appears, for:**
     - tree toggle;
     - sort change;
     - debounced filter rebuild;
     - collapse.

     Until task 3 lands this equals the single `update` call. Task 3 redefines it as the time
     until the job's swap.
   - **Memory** on `random_deep`, using the existing `heap_bytes` test.
3. **Ceilings follow M4's rule:** 1.5× the CI measurement, rounded up. Tests that are slow today
   get honest CEILING values, so CI stays green while the regression stays visible in the
   printed number. The PRD §7 target is named beside each one, as the file already does.
4. **Record the baseline.** The local and CI numbers go into the Outcome table of this doc and
   into [`m6-perf-rebuild.md`](m6-perf-rebuild.md)'s measurements, replacing the throwaway
   profiler's figures.
5. **Runtime.** The whole perf job must stay under about 5 minutes in CI. If the random fixtures
   make it too slow, build each fixture once per test, never more than once.
6. **No product code changes.**

## Files touched

| File | Change |
|---|---|
| `crates/core/tests/perf.rs` | fixtures, new measurements, re-baselined ceilings, an updated header comment |
| `.github/workflows/ci.yml` | only if the perf job's time limit needs raising |
| `docs/plans/m6-perf-rebuild.md` | measurements table replaced with the harness's figures |

## Testing

- Run the perf suite locally (`--release --ignored --nocapture --test-threads=1`) twice, and
  report the medians.
- Paste CI's numbers into the PR, then set the ceilings from them.

## Out of scope

Any speedup. This task only measures.

## Outcome

Done in `crates/core/tests/perf.rs`; no product code changed. The suite grew from 13 to 43 tests
and runs in about 40 s locally and 32 s of test time (59 s job) on CI.

- **Fixtures** `sorted`, `random_flat` and `random_deep` are built with a fixed-seed splitmix64
  shuffle, once per process and cloned per test (the memory test builds fresh, because
  `heap_bytes` counts capacity and a clone is exact-fit).
- **Existing tests** keep their names and ceilings on `sorted` and gain a `_random_deep` twin.
  `random_flat` runs for the rebuild-heavy tests only (sort, tree toggle, debounced rebuild, fold,
  collapse, expand).
- **New:** collapse, expand (through `update` with Left and Right on row 0), fold alone, Name sort
  alone, whole scan on `random_deep` (worst page, total, count of pages over 16 ms), and four
  `time_to_new_list_*` tests (tree toggle, sort change, debounced filter rebuild, collapse). Today
  each is the single `update`; task 3 redefines them as time until the swap.
- **Ceilings** for every random-fixture CEILING are 1.5x PR #78's CI figure, rounded up. On these
  fixtures the CI runner is *faster* than the Apple Silicon development machine (0.7-0.9x), so a
  local run can sit at a ceiling: locally the `sorted` fold-alone test (CI 24 ms, ceiling 37 ms,
  local 38 ms) trips it. CI is the arbiter. Scroll, keystroke, narrowing, scan-batch and render
  stay AT TARGET at 16 ms.
- **Surprise:** `filter first character` on `random_deep` is 22 ms, over 16 ms, so it is a
  CEILING (33 ms) there and AT TARGET only on the sorted control.
- **Memory** on `random_deep` is 99.4 MB (LoadedSet 56.0, View 7.6, Tree 35.8) against 83.4 MB
  sorted, inside the 250 MB budget; the longer names account for the difference.

### Measurements (ms, local mean of 2 runs / CI)

| Measurement (ms) | sorted local / CI | random_flat local / CI | random_deep local / CI |
|---|---|---|---|
| Name sort alone (KeyView::rebuild, no filter) | 4.22 / 3.92 | 241 / 205 | 280 / 228 |
| ScanBatch(500) into 1M, flat scan order | 1.71 / 3.94 | - | 1.63 / 1.22 |
| ScanBatch(500) into 1M, flat sorted by name | 1.37 / 2.49 | - | 1.42 / 1.08 |
| ScanBatch(500) into 1M, tree mode | 1.62 / 3.43 | - | 1.52 / 2.16 |
| collapse one top-level group | - | 386 / 263 | 473 / 372 |
| expand one top-level group | - | 392 / 327 | 510 / 390 |
| filter backspace keystroke (rebuild deferred) | 0.04 / 0.06 | - | 0.05 / 0.07 |
| filter first character | 3.89 / 3.20 | - | 22 / 22 |
| filter keystroke | 0.27 / 0.25 | - | 0.56 / 2.18 |
| filter rebuild after debounce | 22 / 18 | 23 / 19 | 40 / 37 |
| fold alone (Tree::rebuild over a name-sorted view) | 38 / 24 | 151 / 107 | 221 / 165 |
| render alone | 0.04 / 0.04 | - | 0.05 / 0.05 |
| scroll keystroke | 0.04 / 0.07 | - | 0.05 / 0.08 |
| sort change (scan -> name) | 4.23 / 4.45 | 242 / 218 | 278 / 225 |
| time-to-new-list, collapse | - | - | 478 / 360 |
| time-to-new-list, debounced filter rebuild | - | - | 40 / 39 |
| time-to-new-list, sort change | - | - | 281 / 227 |
| time-to-new-list, tree toggle | - | - | 499 / 387 |
| toggle tree mode | 44 / 33 | 392 / 304 | 501 / 386 |
| whole scan, tree mode: worst page | 31.5 / 23.1 | - | 411 / 327 |
| whole scan, tree mode: total | 241 / 182 | - | 1829 / 1285 |
| memory LoadedSet+View+Tree (MB) | 83.4 / 83.4 | - | 99.4 / 99.4 |

### Ceilings (CI x 1.5, rounded up, ms)

| Test (random_deep unless noted) | Ceiling |
|---|---|
| Name sort alone | 342 (flat 308) |
| Sort change | 338 (flat 327) |
| Tree toggle | 580 (flat 456) |
| Collapse / expand | 558 / 586 (flat 395 / 491) |
| Fold alone | 248 (flat 161); sorted 37 |
| Filter first character | 33 |
| Debounced filter rebuild | 57 (flat 28) |
| time-to-new-list: toggle / sort / filter / collapse | 582 / 341 / 58 / 540 |
| Whole scan worst page / total gate | 491 / 2 s |

`sorted` controls keep their M4 ceilings (debounce 49, tree toggle 89, whole-scan worst page 68).

### Findings for tasks 2-5

1. **Collapse costs as much as a tree toggle** (386-473 ms vs 392-501 ms locally): it is a full
   re-filter, re-sort and re-fold. Task 2's fold-only path removes the sort and so should leave
   only the fold (about 165-220 ms), which task 5 then has to shrink.
2. **Sort and fold split roughly 55/45 on random_deep** (280 vs 221 ms locally; 228 vs 165 ms on
   CI), and are about 65x (sort) and 6x (fold) their sorted-control cost. The fold alone is not cheap at random
   arena order, so task 5's gather pass matters as much as task 4's sort.
3. **The whole-scan worst page is 411 ms (327 ms CI)** at 867,500 keys, 12 pages exceed 16 ms,
   and the scan total is 1.8 s (1.3 s CI), so the scan itself is fine: the problem is purely the
   late scheduled rebuilds that task 3 must slice.
