# M6 task 1: Honest harness

Status: **planned.**

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
