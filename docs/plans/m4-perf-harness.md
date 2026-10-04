# M4 task 2: Measure first

Status: **done.**

## Context

PLAN.md M4 row 2: "Measure first · Proves: each PRD §7 metric that can be measured has a
repeatable check: release binary <20 MB as a CI step on the release profile; `update` + render at
1M keys within 16 ms (scroll, filter keystroke, sort); Loaded-set plus view memory at 1M keys
inside 250 MB (core-side accounting); first `ScanBatch` <150 ms and interactive <1 s at 100k keys,
as an integration timing test. The release profile gets `strip`. Baseline numbers are recorded in
the doc before tasks 3–4, so the wins are measured, not claimed."

This task exists because none of PRD §7's metrics are checked anywhere today:

- **No benches, no criterion.** `crates/core/examples/memreport.rs` prints heap bytes for
  100k/1M synthetic keys but asserts nothing — it is a manual tool, run by eye. `crates/app/examples/frame.rs`
  renders real frames against a real server and prints timings in its own doc comment's example
  usage, also asserting nothing.
- **Two tests touch the budget but neither is a CI gate on the real numbers:**
  `crates/core/tests/golden.rs`'s `rendering_cost_does_not_grow_with_the_keyspace` builds a
  200k-key `LoadedSet` and renders, in the **debug** profile (the default `cargo test` run), so its
  timing (if it asserts one — confirm by reading the test directly before building this task's CI
  step) is not representative of a release build. `crates/core/src/state/loaded.rs`'s
  `a_million_keys_fit_inside_the_budget` checks `heap_bytes()` only — the `LoadedSet` arena itself,
  not the `View`'s `order: Vec<u32>` index or the `Tree`'s per-key allocations (see
  `m4-perf-interaction.md`), so it is not yet the full "Loaded-set plus view memory" PLAN's row 2
  asks for.
- **`Cargo.toml` has exactly one custom profile**, `[profile.dist]` (`inherits = "release"`, `lto =
  "thin"`), with **no `strip`**. The ordinary `release` profile is plain Cargo defaults.
- **CI (`.github/workflows/ci.yml`) runs three jobs, all `runs-on: ubuntu-latest`**, and none of
  them builds in release mode or measures binary size.
- **The local release binary, built from this branch, is 10,445,072 bytes (~10.0 MiB)** — already
  well inside the <20MB target, which is good news but has never been asserted anywhere a
  regression would be caught.

This task is ordered before tasks 3 and 4 deliberately (PLAN's risk order, item 2): without a
baseline and a release-mode check, "task 3/4 made it faster" is a claim nobody can verify, and a
regression in task 3 or 4 would not be caught until someone noticed the app feel slow by hand.

## Decisions

1. **A size-check CI job building `--profile dist` (or `release` with `strip` added) and failing
   above 20 MB.** `strip = true` (or `"symbols"`, the finer-grained option — **confirm at build
   time** which is more appropriate; `"symbols"` keeps enough for a readable backtrace in some
   toolchains, `true` is simplest) is added to `[profile.dist]` so the shipped artifact gets both
   `lto = "thin"` and stripping; the CI job builds that profile and checks the resulting binary's
   size with a plain `stat`/`ls -l`, not a separate tool.
2. **Release-mode budget tests are gated behind `#[ignore]` plus a separate CI job that runs them
   with `--release`,** so `cargo test --workspace` (the default, Docker-free, debug-profile run)
   stays fast and the existing integration-suite pattern (`#[ignore]`d, run by a dedicated job) is
   reused rather than inventing a second mechanism. This mirrors the integration suite's own
   shape: `cargo test -p redis-pane-core --release -- --ignored` (or equivalent) as a new CI job,
   separate from both the default suite and the Docker-backed integration job.
3. **What gets measured, and where:**
   - **Binary size** — the new size-check CI job, release/dist profile, hard-coded 20MB ceiling.
   - **`update` + render at 1M keys within 16ms** (scroll, filter keystroke, sort) — a new
     release-mode-only benchmark-shaped test, most naturally living beside
     `rendering_cost_does_not_grow_with_the_keyspace` in `crates/core/tests/golden.rs` or a new
     `crates/core/tests/perf.rs`, timing `update()` calls (not just render, since a filter
     keystroke's cost is dominated by `rebuild_list`, which is `update`-side) against a 1M-key
     `State`. **Confirm at build time** whether this lives in `golden.rs` (keeping one
     performance-test file) or a new `perf.rs` (separating "does it look right" from "is it fast");
     a new file is probably cleaner once task 3/4 add several more of these cases.
   - **Loaded-set plus view memory at 1M keys inside 250MB** — extend `a_million_keys_fit_inside_the_budget`'s
     style of assertion (`heap_bytes()`-shaped, deterministic, no RSS measurement) to also account
     for `View.order: Vec<u32>` (4MB at 1M keys, already accounted for in ADR-0010's own math but
     not asserted) and `Tree`'s allocations once task 4 changes its representation — this task
     establishes the *pattern and the budget check*; task 4's doc is where the Tree-specific
     accounting actually changes.
   - **First `ScanBatch` <150ms and interactive <1s at 100k keys** — an integration timing test
     (needs Docker, since it is timing a real `SCAN` round trip, not a synthetic benchmark),
     `#[ignore]`d like the rest of `crates/app/tests/integration.rs`, run by the existing
     integration CI job (it already runs on every push/PR/nightly) rather than a new one, since it
     needs the same container-per-test machinery that suite already has.
4. **Baseline numbers, measured against this branch** (machine: Apple Silicon, local; CI numbers
   land once the new jobs run there — this table is the local baseline the override rule in the
   build task compared every ceiling against):

   | Metric | Baseline (measured) | After task 3 | PRD §7 target | Gate used | Release CI gate? |
   |---|---|---|---|---|---|
   | Binary size (dist profile, stripped) | 8,607,904 bytes (~8.21 MiB) | unchanged (task 3 touches no binary-size-relevant dependency) | <20MB | AT TARGET: <20MB | yes, new `size` job |
   | `update`+render, scroll keystroke, 1M keys | 42.7µs | 75.5–76.1µs (noise; scrolling touches no arena, sort or rebuild either before or after task 3) | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `update`+render, filter keystroke, 1M keys | 22.27ms (CI: 31.2ms) | 23.8–26.2ms — unchanged in shape: `KeyView::rebuild` on every keystroke is M4 task 4's scope, not task 3's (task 3's scope is `scan_batch`'s per-*page* cost, explicitly not interactive filter/tree edits) | <16ms | CEILING: <47ms (CI baseline × 1.5 — CI is the slower machine here), tightened by task 4 | yes, `--release`, `#[ignore]`d |
   | `update`+render, sort change, 1M keys | 3.76–3.82ms | 3.99–4.07ms (noise; untouched by task 3) | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `update`+render, toggle tree mode, 1M keys | 124–129ms | 129–132ms (noise; `Tree::rebuild`'s own cost is task 4's scope, not task 3's) | <16ms | CEILING: <194ms (baseline × 1.5), tightened by task 4 | yes, `--release`, `#[ignore]`d |
   | `update`+render, `ScanBatch` of 500 into 1M keys — flat, scan order | 2.29–2.52ms | **1.1–1.5ms** — now `KeyView::extend`'s `O(page)` path (decision 1) rather than a full `rebuild_list` | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `update`+render, `ScanBatch` of 500 into 1M keys — **tree mode (the default view)** | 129.6–130.7ms | **~1.0–1.1ms** — the geometric schedule (decision 2) skips the rebuild entirely for a page this far from the next 25%-growth threshold | n/a (16ms bar) | AT TARGET: <16ms (down from CEILING 195ms) | yes, `--release`, `#[ignore]`d |
   | `update`+render, `ScanBatch` of 500 into 1M keys — flat, sorted by Name | 6.79–7.01ms | **~1.0–2.0ms** — same reason as the tree-mode row above | n/a (16ms bar) | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | Whole scan, 2,000 pages of 500, folded into the core one page at a time, **tree mode from empty** — total | 120.4–121.8s | **~0.58–0.79s** — ~150–200× faster; no longer trips the 240s wall cap | not named by PRD §7 directly; recorded for task 3/4's before/after | AT TARGET: <10s (task 3's own "well under 10s" target; was observed, not asserted) | yes, `--release`, `#[ignore]`d, **no longer `--skip`ped** by the `perf` CI job |
   | Same run — median single page | not measured (the pre-task-3 code had no cheap pages to make a median meaningful) | **~0.01ms** — new row: proof the geometric schedule, not merely a faster rebuild, fixed the common case | <16ms (one frame) | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | Same run — worst single page | 137–251ms | **~100–145ms** — still above budget: the page that lands on a scheduled rebuild still pays `rebuild_list`'s own Name-sort-plus-`Tree::rebuild` cost (M4 task 4's scope) | <16ms (one frame) | CEILING: <220ms (baseline × 1.5, from the 145ms run; was <378ms), tightened by task 4 | yes, `--release`, `#[ignore]`d |
   | Render alone, 1M keys | 40.4–42.6µs | 62–73µs (noise; untouched by task 3) | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `LoadedSet` + `KeyView.order` + `Tree` heap bytes, 1M keys | 75.8MB (40.0 + 3.8 + 32.0) | unchanged — task 3 changes when a rebuild runs, not what it allocates | <250MB RSS (half-budget margin, matching the existing arena-only test) | AT TARGET: <125MB | yes, `--release`, `#[ignore]`d (pure arithmetic, debug-safe too) |
   | First `ScanBatch`, 100k keys | 1.93–2.08ms (loopback Docker, debug build) | not re-measured (unaffected: this is a real `SCAN` round trip, not `scan_batch`'s view-update cost) | <150ms | AT TARGET: <150ms | yes, integration job (Docker) |
   | Interactive (first batch folded + a frame rendered), 100k keys | 2.70–2.92ms (loopback Docker, debug build) | not re-measured (same reason) | <1s | AT TARGET: <1s | yes, integration job (Docker) |

   **Revised picture after measuring tree mode specifically** (tree mode is the app's *default*
   view — the first cut of this table measured only the flat/scan-order case, which understated
   the real cost): the earlier note below that `ScanBatch`-into-1M "already lands inside the
   16ms bar" was true only in flat scan order. In tree mode — what a fresh launch actually runs —
   the same single page costs **~18× more** (130ms vs 7ms), because `rebuild_list` in tree mode
   pays for a full Name sort *and* a full `Tree::rebuild` on every page, not just the final one.
   The whole-scan-fold test confirms the O(n²) the plan predicted: each page's cost is
   proportional to the keys already loaded, so 2,000 pages cost roughly 2,000 × (130 ms ÷ 2) ≈
   130 s — and ~120 s was measured locally. The worst single page (observed near the 900–995k
   mark, with some run-to-run jitter) cost 137–251 ms. The user-visible result is the real
   finding: about two minutes of CPU to finish scanning 1M keys in the default view, during which
   every page blocks the UI for over 100 ms.

   **CI numbers** (ubuntu-latest, first run on PR #58): scroll 0.10 ms, sort change 5.4 ms, render
   0.06 ms, scan page flat 3.8 ms / name-sorted 8.0 ms / tree mode 60.7 ms, tree toggle 60.2 ms,
   filter keystroke 31.2 ms. CI is slower than the local machine on the flat paths and faster on
   the tree paths. Only the filter ceiling needed raising (34 → 47 ms, from CI's own baseline);
   every other gate has at least 2× headroom on both machines.

   Of the thirteen timing/memory metrics above, nine were already at the PRD §7 target before any
   fix landed. Four sat on a regression ceiling — the filter keystroke, toggling tree mode, a
   `ScanBatch` in tree mode, and the whole-scan fold's worst page — from two causes:
   `rebuild_list`'s full-keyspace rescan on every scan page (task 3's scope) and `Tree::rebuild`'s
   own per-rebuild cost (task 4's scope).

   **After task 3** (see the "After task 3" column above, and
   [`m4-perf-scan.md`](m4-perf-scan.md) for the resolved decisions): the `ScanBatch`-in-tree-mode
   and flat-sorted-by-name cases are now `AT TARGET`, and the whole-scan fold dropped from ~120s to
   well under a second — both because `scan_batch` no longer rebuilds the full view on every page
   (an `O(page)` incremental append in the common flat/scan-order case, a geometric rebuild
   schedule otherwise). What is left above budget — the filter keystroke, toggling tree mode, and
   the worst single page of a scan (the one that does land on a scheduled rebuild) — is entirely
   `rebuild_list`'s own Name-sort-plus-`Tree::rebuild` cost, which is M4 task 4's scope. Tree mode
   remains where task 4 will be judged.

## Architecture

Core-side: the `update`/render timing tests and the memory-accounting test are pure
`redis-pane-core` tests — no I/O, consistent with CLAUDE.md's functional-core discipline; timing a
pure function in a test is not the kind of I/O the "render loop never does I/O" rule is about, but
the test must still build its 1M-key `State` deterministically (same synthetic-key generator
`memreport.rs` already uses) so it is reproducible across machines, with a generous-enough budget
that CI runner variance does not flake it — **confirm at build time** whether the 16ms figure
needs headroom in CI (e.g. asserting <32ms there while documenting 16ms as the interactive target
on real hardware) or whether it holds as-is; this is exactly the kind of number that should come
from the baseline run, not be guessed here.

Shell-side: the `[profile.dist]` change and the two new CI jobs (size check, release-mode perf
gate) are pure CI/build configuration, no app code.

## Files touched

| File | Change |
|---|---|
| `Cargo.toml` | `[profile.dist]` gains `strip` |
| `crates/core/tests/golden.rs` or new `crates/core/tests/perf.rs` | 1M-key `update`/render timing tests, `#[ignore]`d |
| `crates/core/src/state/loaded.rs` or a new test alongside it | extended memory-budget assertion including `View.order` |
| `.github/workflows/ci.yml` | new size-check job (dist profile build + size assertion); new release-mode perf-gate job (`--release -- --ignored`) |
| `crates/app/tests/integration.rs` | new `#[ignore]`d timing test for first-`ScanBatch`/interactive-at-100k, run by the existing integration job |
| this doc | baseline numbers table, filled in during the build |

## Testing

This task *is* the testing infrastructure — see Decisions §3 for what each new check covers.
Confirm the existing `rendering_cost_does_not_grow_with_the_keyspace` test's current assertion
(read it directly during phase 1) before deciding whether it is extended in place or superseded
by the new 1M-key cases.

## CLAUDE.md rules this binds

- **"1M keys of average length occupy 40MB" / "measured"** — this task is the first place that
  measured claim (currently only exercised via `memreport.rs`, by eye) becomes an asserted,
  CI-enforced budget alongside the `View`'s own memory.
- **The render loop never does I/O** — the 1M-key perf tests exercise `update()` and `render()`
  directly against a synthetic `State`, never a real connection, keeping them fast and
  Docker-free except for the one integration timing case that is inherently about real `SCAN`
  latency.

## Out of scope

- **Actually fixing any of the measured hot paths** — that was tasks 3 and 4's job, not this
  task's (task 2 only proved what the current numbers were and wired the gates that would catch a
  regression or confirm an improvement; task 3 is now done, task 4 is not started).
- **A general benchmarking framework (criterion, etc.)** — the budgets here are pass/fail
  assertions against fixed targets, not a trend-tracking benchmark suite; nothing in PRD §7 or the
  M4 scope asks for historical trend data.
