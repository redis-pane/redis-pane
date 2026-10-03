# M4 task 2: Measure first

Status: **planning — not started.**

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
4. **Baseline numbers are recorded in this doc once the harness exists and has run once against
   this branch — not invented now.** The table below is a placeholder to be filled in during the
   build, before task 3 touches `scan_batch` or `rebuild_list`:

   | Metric | Baseline (fill in) | Budget | Release CI gate? |
   |---|---|---|---|
   | Binary size (dist profile, stripped) | — | <20MB | yes, new job |
   | `update()` on a filter keystroke, 1M keys | — | <16ms | yes, `--release`, `#[ignore]`d |
   | `update()` on a sort change, 1M keys | — | <16ms | yes, `--release`, `#[ignore]`d |
   | Render, 1M keys (existing 200k case extended) | — | <16ms | yes, `--release`, `#[ignore]`d |
   | `LoadedSet` + `View.order` heap bytes, 1M keys | — | <250MB RSS equivalent | yes, debug-safe (pure arithmetic) |
   | First `ScanBatch`, 100k keys | — | <150ms | yes, integration job (Docker) |
   | Interactive (list usable), 100k keys | — | <1s | yes, integration job (Docker) |

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
