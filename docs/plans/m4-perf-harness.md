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

   | Metric | Baseline (measured) | PRD §7 target | Gate used | Release CI gate? |
   |---|---|---|---|---|
   | Binary size (dist profile, stripped) | 8,607,904 bytes (~8.21 MiB) | <20MB | AT TARGET: <20MB | yes, new `size` job |
   | `update`+render, scroll keystroke, 1M keys | 42.7µs | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `update`+render, filter keystroke, 1M keys | 22.27ms (CI: 31.2ms) | <16ms | CEILING: <47ms (CI baseline × 1.5 — CI is the slower machine here), tightened by task 3 | yes, `--release`, `#[ignore]`d |
   | `update`+render, sort change, 1M keys | 3.76–3.82ms | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `update`+render, toggle tree mode, 1M keys | 124–129ms | <16ms | CEILING: <194ms (baseline × 1.5), tightened by task 4 | yes, `--release`, `#[ignore]`d |
   | `update`+render, `ScanBatch` of 500 into 1M keys — flat, scan order | 2.29–2.52ms | n/a (held to the 16ms bar every update pays) | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `update`+render, `ScanBatch` of 500 into 1M keys — **tree mode (the default view)** | 129.6–130.7ms | n/a (16ms bar) | CEILING: <195ms (baseline × 1.5), tightened by task 3/4 | yes, `--release`, `#[ignore]`d |
   | `update`+render, `ScanBatch` of 500 into 1M keys — flat, sorted by Name | 6.79–7.01ms | n/a (16ms bar) | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | Whole scan, 2,000 pages of 500, folded into the core one page at a time, **tree mode from empty** — total | 120.4–121.8s | not named by PRD §7 directly; recorded for task 3/4's before/after | observed, not asserted | yes, `--release`, `#[ignore]`d (wall-capped at 240s; both runs finished uncapped) |
   | Same run — worst single page | 137–251ms | <16ms (one frame) | CEILING: <378ms (baseline × 1.5, from the 251ms run), tightened by task 3/4 | yes, `--release`, `#[ignore]`d |
   | Render alone, 1M keys | 40.4–42.6µs | <16ms | AT TARGET: <16ms | yes, `--release`, `#[ignore]`d |
   | `LoadedSet` + `KeyView.order` + `Tree` heap bytes, 1M keys | 75.8MB (40.0 + 3.8 + 32.0) | <250MB RSS (half-budget margin, matching the existing arena-only test) | AT TARGET: <125MB | yes, `--release`, `#[ignore]`d (pure arithmetic, debug-safe too) |
   | First `ScanBatch`, 100k keys | 1.93–2.08ms (loopback Docker, debug build) | <150ms | AT TARGET: <150ms | yes, integration job (Docker) |
   | Interactive (first batch folded + a frame rendered), 100k keys | 2.70–2.92ms (loopback Docker, debug build) | <1s | AT TARGET: <1s | yes, integration job (Docker) |

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

   Of the thirteen timing/memory metrics above, nine are already at the PRD §7 target. Four sit on
   a regression ceiling — the filter keystroke, toggling tree mode, a `ScanBatch` in tree mode, and
   the whole-scan fold's worst page — and all four come from the same two causes:
   `rebuild_list`'s full-keyspace rescan (task 3) and `Tree::rebuild`'s full rebuild on every page
   (task 4). Tree mode is where both tasks will be judged.

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

- **Actually fixing any of the measured hot paths** — that is tasks 3 and 4. This task only
  proves what the current numbers are and wires the gates that will catch a regression or confirm
  an improvement.
- **A general benchmarking framework (criterion, etc.)** — the budgets here are pass/fail
  assertions against fixed targets, not a trend-tracking benchmark suite; nothing in PRD §7 or the
  M4 scope asks for historical trend data.
