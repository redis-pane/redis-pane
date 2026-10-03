# M4 — Scale & polish: task plan and per-task docs

## Amendment (2026-10-03): Cluster becomes M5

After the Cluster analysis, the user chose to make **Cluster its own milestone after the beta
(M5)**. M4 still opens with the refusal (task 1), which is the safe state until M5 lands. Changes
to the open planning PR #54, on the `m4-planning` branch, before it merges:

- **`docs/plans/m5-cluster.md` (new):** the Cluster design from the 2026-10-03 analysis, so it
  isn't lost:
  - what a Cluster changes (slots, per-node commands, per-node tracking, db 0 only, `CROSSSLOT`,
    topology changes);
  - the design for each feature:
    - title bar `cluster · N primaries`, and `redis-cli -c` for copied commands;
    - a merged N-cursor scan with progress summed over primaries, and "the cluster changed during
      the scan — r to rescan" on topology change;
    - owner-pinned `CLIENT CACHING` (arming and read pinned to the key's slot), re-armed only when
      the owner node reconnects, and re-armed on the new owner when a key moves;
    - a `CROSSSLOT` guard for rename and copy, bulk delete grouped by node, and move-across-db
      hidden;
    - Dashboard aggregated by default with a node table and `Enter` to drill into one node, which
      is a drill-down inside a view, not a connection switch, so ADR-0005 holds;
    - a merged Slowlog with a NODE column, where reset confirms "on N nodes";
    - Monitor merged across primaries, with a cost banner naming N;
    - sharded Pub/Sub chips (`SSUBSCRIBE`);
    - the `replica` read-only reason narrowed;
  - the test-harness cost (a real 3-primary cluster in testcontainers, and the announce-address
    pitfalls);
  - the pros and cons;
  - stages 1 to 4 as M5's task list (stage 0, the refusal, is M4 task 1).
- **`docs/PRD.md`:** add an **M5 — Cluster** milestone after M4. The Cluster non-goal and R1.11
  now point to M5, not "past M4".
- **`docs/PLAN.md`:** §7 and §8 say Cluster is M5 (`m5-cluster.md`), not "deferred indefinitely".
- **ADR-0021:** the decision reads "refused at startup until M5 delivers support". Link
  `m5-cluster.md`.
- **M4 task 1 ([`m4-cluster-refusal.md`](m4-cluster-refusal.md)):** the diagnostic wording points to M5. Scope unchanged.

Everything below stands as approved.

## Context

M3 is complete (alpha.18). The PRD defines M4 as "Cluster support, million-key performance
work, themes, packaging and distribution", but none of it is planned: no task table in PLAN.md,
no `docs/plans/m4-*.md`. Research into the current code found:

- **Cluster quietly misbehaves today.** A `redis-cluster://` URL connects and the app:
  - scans one arbitrary primary, then reports the scan complete;
  - sends `CLIENT CACHING YES` to a random node rather than the key's owner, yet still shows
    `● live`;
  - shows one node's view in the Dashboard and Slowlog, silently (Pub/Sub sees classic
    `PUBLISH` cluster-wide, but drops sharded `SPUBLISH` messages).

  Only Monitor refuses (`feed.rs` `monitor_config`). DESIGN.md links a nonexistent
  `adr/0008-cluster-support.md`.
- **Performance is unmeasured, and there are known hot paths:**
  - No PRD §7 metric is checked in CI.
  - The release binary is ~10 MB with no `strip`.
  - `scan_batch` calls `rebuild_list` after every 500-key page (~2,000 full rebuilds at 1M,
    each with a full sort), so the cost grows with the square of the keyspace.
  - The filter rebuilds, re-sorts and rebuilds the tree on every keystroke.
  - The tree allocates per key and per segment.
  - `row_of` is a linear scan.
  - Metadata fetches have no dedup or cancel.
  - A real bug: `LoadedSet::clear` doesn't clear `ttl_read_at`, so it grows on every rescan.
  - The shell redraws after every `Msg`.
- **Themes:** only one hard-coded dark palette. DESIGN promises a light default, a high-contrast
  option and user themes as data; ADR-0011 promises light-theme goldens.
- **Glyphs:** no ASCII fallback exists (CLAUDE.md requires width-identical Nerd Font/ASCII sets).
- **Session restore:** the state file (ADR-0003, DESIGN §7) doesn't exist (`state_file.rs`
  absent).

**Scope decisions (user, 2026-10-01):**
- **Cluster: refuse clearly and defer past M4.** Replace the silent misbehaviour with an explicit
  startup refusal. Real support is no longer M4.
- **Packaging:** all four items are wanted (Homebrew and release notes, musl and arm64 Linux,
  macOS signing and notarization, Windows signing) but decided **at the end of alpha**. M4 ships
  no official packages.
- **Polish:** light and high-contrast themes plus user themes; session restore; ASCII glyph
  fallback. Keybinding overrides from config are **not** in M4.
- **M4 ends with a beta** (`0.1.0-beta.1`), not 1.0.

## Deliverable 1: planning docs (one PR, docs only, like M3's #42)

- **`docs/PLAN.md`:** a new `## 7. M4 — Scale & polish` section with the task table below.
  Renumber "Explicitly not in…" to §8 and "Risk order" to §9, and add the M4 risk order.
- **`docs/PRD.md`:** rewrite the §9 M4 bullet to the decided scope.
  - Cluster moves past M4: §3 non-goal wording and R1.11 note.
  - Packaging becomes "decided at the end of alpha", listing the four items.
  - M4 ends in a beta.
- **`docs/adr/0021-cluster-refused-until-supported.md`** (new). It amends ADR-0008:
  - Cluster stays out past M4.
  - Until then a Cluster target is refused at startup with a clear message, because silently
    serving one node's view is worse than refusing. ADR-0008 rejected "Cluster with a
    single-node dashboard" for the same reason.
  - It lists what real support must solve (ADR-0008's three items, plus the `CLIENT CACHING`
    routing finding).
- **One `docs/plans/m4-*.md` per task**, shaped like the M3 docs: context, decisions with
  recommended defaults flagged to confirm at build time, files touched, tests, and the CLAUDE.md
  rules it binds.
- Fix DESIGN.md's broken `adr/0008-cluster-support.md` link.

## M4 task table (goes into PLAN.md §7)

| # | Task | Proves |
|---|---|---|
| 1 | **Refuse Cluster targets** ([`m4-cluster-refusal.md`](m4-cluster-refusal.md)) | A `redis-cluster://` URL or a Cluster-mode server (detected via `INFO cluster` `cluster_enabled:1`, so a plain `redis://` URL to a cluster node is caught too) fails at startup with a startup diagnostic naming ADR-0021, never a half-working session. `infer_environment` recognises `-sentinel`/`-cluster` schemes, so a loopback Sentinel URL is `local`, not `unknown`. Integration test against a real `cluster-enabled yes` container. |
| 2 | **Measure first** ([`m4-perf-harness.md`](m4-perf-harness.md)) | Each PRD §7 metric that can be measured has a repeatable check:<br>• release binary <20 MB, as a CI step on the release profile;<br>• `update` + render at 1M keys within 16 ms (scroll, filter keystroke, sort);<br>• Loaded-set plus view memory at 1M keys inside 250 MB (core-side accounting);<br>• first `ScanBatch` <150 ms and interactive <1 s at 100k keys, as an integration timing test.<br>The release profile gets `strip` and `lto`. The baseline numbers are recorded in the doc *before* tasks 3–4, so the wins are measured, not claimed. |
| 3 | **Million-key scan** ([`m4-perf-scan.md`](m4-perf-scan.md)) | Scanning 1M keys does O(n) total list work, not O(n²): the view extends incrementally per page in scan order, and a full re-sort runs only when a non-scan sort or tree mode is active, deferred or batched. The shell coalesces redraws (one frame per tick, not per `Msg`). The `ttl_read_at` leak is fixed, with a regression test. Task 2's budgets pass at 1M. |
| 4 | **Million-key interaction** ([`m4-perf-interaction.md`](m4-perf-interaction.md)) | A filter keystroke at 1M keys meets the 16 ms frame budget: it narrows from the previous result when the query extends, and otherwise rebuilds once, debounced. The tree rebuild allocates no per-key `String` or `Vec`, and collapsed-group lookup is O(1). `row_of` uses an inverse index. Metadata fetches are deduplicated and the stale ones cancelled. Budgets pass. |
| 5 | **Themes** ([`m4-themes.md`](m4-themes.md)) | `Theme` becomes data: a token→colour palette per colour depth. Built-ins are dark (today's), light and high-contrast, all checked for WCAG AA contrast by a test. The theme is chosen by an additive config field `theme` and a `--theme` flag. User themes are token→colour maps in config, unknown tokens a parse error. Goldens exist for light and high-contrast (ADR-0011's promise). `NO_COLOR` treats an empty value as unset, per no-color.org. |
| 6 | **ASCII glyph fallback** ([`m4-glyphs.md`](m4-glyphs.md)) | Every hard-coded glyph (`✕ ⊘ ✎ ▌ ● ○ ⁎ ⟡ ▶`, sparkline and bar blocks, box drawing where needed) goes through one glyph set with a Unicode and an ASCII variant. A test enforces width-identical variants. ASCII is chosen by config or flag, or automatically when the locale isn't UTF-8. Goldens exist in ASCII mode. |
| 7 | **Session restore** ([`m4-session-restore.md`](m4-session-restore.md)) | Per ADR-0003, a state file at `$XDG_STATE_HOME/redis-pane/state.json`, keyed per target (no secrets), restores pane split, tree/flat, sort, filter and the selected key on relaunch. The app still never writes config. A corrupt or old file is ignored with a notice, never a crash. Writes are atomic (temp file plus rename) and happen on quit and on change, debounced. |
| 8 | **Close out M4 → beta** | PLAN/PRD marked complete. ALPHA.md becomes beta notes (what to try covers themes, glyphs, restore). README status says beta. Release `0.1.0-beta.1`. Packaging remains the open end-of-alpha decision, recorded in PRD §10. |

**Risk order:**
1. Task 1, because it's a live correctness and safety gap today.
2. Task 2, before 3 and 4, so optimisations are proven against a baseline.
3. Task 5 before 6, because both reshape how render asks for presentation and 6 reuses 5's
   "presentation as data" pattern.
4. Task 7, which is independent.

## Execution (after the planning-docs PR merges)

The same workflow as M3. Each task gets its own branch and PR. A Sonnet subagent builds it in
phases from its `docs/plans/m4-*.md`, and I review between phases. I re-run every check myself
before opening the PR, and merge only after CI is green and you've had a chance to try it. A
release follows a task or group when you ask for one; `0.1.0-beta.1` closes M4.

## Verification

- **Planning PR:**
  - PLAN.md's M4 table uses the same columns as M0–M3.
  - Every row links an existing `docs/plans/m4-*.md` whose decisions match the row.
  - ADR-0021 is linked from ADR-0008, the PRD and DESIGN.
  - The broken ADR link is fixed.
  - It's docs only, so no tests change.
- **Each task:**
  - `cargo test --workspace`, clippy `-D warnings`, fmt check, and the boundary check.
  - The full integration suite, time-capped.
  - Task 2's budgets from then on.
  - Golden diffs reviewed.
  - A by-hand script in each PR, e.g. `fixtures.py -n 1000000` for tasks 3–4, `--theme light`
    for 5, `LANG=C` for 6, and relaunch for 7.
