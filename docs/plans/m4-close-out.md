# M4 task 8: Close out M4 → beta

Status: **planning — not started.**

## Context

PLAN.md M4 row 8: "Close out M4 → beta · Proves: PLAN/PRD marked complete. `ALPHA.md` becomes beta
notes (what to try covers themes, glyphs, restore). README status says beta. Release
`0.1.0-beta.1`. Packaging remains the open end-of-alpha decision, recorded in PRD §10."

This task is the mechanical close-out M3's own PLAN.md §6 progress note already modeled (
"**Progress: done.** All six tasks are built...", with a narrative paragraph), applied to M4, plus
the version bump and the two user-facing docs (`README.md`, `ALPHA.md`) that describe the product's
current state to someone arriving fresh. The facts this task's edits need to land against:

- **Current version**: `Cargo.toml`'s `[workspace.package] version = "0.1.0-alpha.18"`, inherited
  by both crates via `version.workspace = true`. M3's close-out commit (per the git log:
  `88bfe63 Close out M3 and bump version to 0.1.0-alpha.18`) already established the pattern this
  task repeats — close out the milestone and bump the version in the same change.
- **`ALPHA.md`'s content is already stale relative to M2/M3**, independent of M4: it opens "It
  cannot write anything. This build is **read-only**. There is no delete, no edit, no rename —
  that work hasn't started yet (it's milestone M2)." M2 is long done (editing, TTL, delete all
  ship) and M3 shipped Monitor/Slowlog/Dashboard/Pub-Sub — none of that is reflected in `ALPHA.md`
  today. `README.md`'s own "Status" section, by contrast, *is* current — it correctly describes
  editing, Monitor, Slowlog and Dashboard as shipped, and says "This is an early alpha." This task's
  README change is narrower than `ALPHA.md`'s (just the status word/phrase changing from alpha to
  beta), while `ALPHA.md` needs a substantially larger content update that is overdue regardless of
  M4 — **this task should fix `ALPHA.md`'s stale M2 framing as part of the same change**, not just
  append M4 content on top of text that already describes a build two milestones out of date.
- **Install URLs** in both `ALPHA.md` and `README.md` are version-pinned
  (`.../releases/download/v0.1.0-alpha.18/...`) and must move to whatever tag `0.1.0-beta.1`
  actually gets.
- **PRD §10's packaging open question** (added by this same planning change — see
  `docs/PRD.md` §10) stays open after M4 closes; this task's job is to make sure it stays
  *recorded* as open, not to resolve it — the end-of-alpha packaging decision is explicitly out of
  M4's scope per the user's 2026-10-01 scope call.

## Decisions

1. **Keep the `ALPHA.md` filename through this close-out**, retitling its *content* to beta notes
   rather than renaming the file. **Confirm at build time** whether the filename itself should
   change (`BETA.md`) once the packaging decision actually lands at the end of alpha — renaming now
   would break every link to `ALPHA.md` from GitHub Release notes, prior PRs, and possibly external
   references, for a file whose *name* ("the alpha-testing notes file") is arguably still accurate
   even once its *content* describes a beta — "alpha" in the filename can be read as "this is where
   I point early testers," a convention, not strictly a lifecycle-stage label. Recommended default:
   keep the filename, retitle the heading and content (e.g. the `# Trying redis-pane (alpha)` H1
   becomes `# Trying redis-pane (beta)`), and revisit the filename specifically at the actual
   end-of-alpha packaging decision, when there may be a cleaner natural renaming point (e.g.
   alongside a packaging README split).
2. **`ALPHA.md`'s "It cannot write anything" section is rewritten** to describe what the beta
   actually can do — edit every core type, TTL, delete (M2), Monitor/Slowlog/Dashboard/Pub-Sub
   (M3), plus whatever M4 adds (themes, ASCII fallback, session restore, the refused-not-silent
   Cluster behavior) — rather than patched to merely add M4's items on top of an M2-era "read-only"
   framing that is no longer true. The "what to try" framing PLAN's row names should specifically
   call out M4's new surface: switching themes (`--theme light`), the ASCII fallback (useful to
   mention explicitly since a tester over a genuinely ASCII-only terminal would otherwise not know
   the option exists), and that their session (split, filter, sort, selected key) now survives a
   relaunch.
3. **`README.md`'s "Status" section** changes "This is an early alpha" to describe a beta, and its
   closing "Rename, copy, and bulk operations across several keys are coming next" line should be
   reconsidered — those are M2 tasks 11–13 (per `docs/PLAN.md` §5), which this M4 close-out does
   not itself complete; **confirm at build time** whether that line stays as-is (still accurate —
   those tasks remain unbuilt) or is updated to reflect whatever M4 actually shipped as "coming
   next" instead (Cluster support, once ADR-0021 is eventually revisited, or the packaging
   decision, are both more relevant "what's next" items once M4 closes than the still-pending M2
   tasks are).
4. **Version bump to `0.1.0-beta.1`** in `Cargo.toml`'s `[workspace.package]`, following the exact
   pattern M3's close-out commit already used (bump alongside the close-out, not as a separate
   change).
5. **PLAN.md §7's "Progress: not started." line becomes a "done" narrative**, in the same shape
   §6's M3 progress paragraph already uses — naming each task, what shipped, and pointing at its
   plan doc, written once the actual M4 work (tasks 1–7) is complete, not by this task alone (this
   task is itself task 8, so its own plan doc gets added to that narrative last).
6. **PRD §9's M4 bullet** (already rewritten by this planning change to describe the M4 scope)
   gets its "ends in a beta" clause confirmed true at close-out — no further rewrite needed unless
   the actual build diverged from what was planned, in which case CLAUDE.md's "when a feature
   diverges from these docs, update the doc in the same change" applies as it would to any task.
7. **PRD §10's packaging open question stays open, explicitly, in the close-out text** — this task
   should not quietly resolve or remove it; a one-line pointer in the close-out commit/PR
   description (not necessarily the PRD itself, which already states it correctly) reiterating that
   packaging remains undecided is enough to satisfy PLAN's "Packaging remains the open end-of-alpha
   decision, recorded in PRD §10" proof clause.

## Architecture

Pure documentation and version-metadata changes — no core/shell architecture questions. The one
mechanical risk is version-string consistency: `Cargo.toml`'s workspace version, `ALPHA.md`'s and
`README.md`'s embedded release-download URLs, and any `--version`-flag output (if the binary prints
its own version, which `version.workspace = true` suggests it does via Cargo's standard
`CARGO_PKG_VERSION`) must all agree after the bump — worth a final grep for `0.1.0-alpha.18`
across the repo before this task is considered done, to catch anything this doc's file list missed.

## Files touched

| File | Change |
|---|---|
| `Cargo.toml` | `[workspace.package] version` → `0.1.0-beta.1` |
| `ALPHA.md` | content rewritten for the beta (filename kept, per decision 1); install URLs re-pinned to the new tag |
| `README.md` | "Status" section says beta; install URLs re-pinned; "coming next" line reviewed |
| `docs/PLAN.md` §7 | "Progress: not started." → a "done" narrative once M4 tasks 1–7 actually ship |
| `docs/PRD.md` §9 | confirmed accurate against what actually shipped (update only if it diverged) |

## Testing

This is a documentation/metadata-only task — no code to test. Verification is: every version
string in the repo agrees after the bump (a grep for the old version string finds nothing); every
install URL in `ALPHA.md`/`README.md` resolves to the actual `0.1.0-beta.1` release tag once it
exists; `docs/PLAN.md` §7's task table rows no longer read "not started" anywhere inconsistent with
the narrative above them.

## CLAUDE.md rules this binds

- **"When a feature diverges from these docs, update the doc in the same change — the docs are the
  spec, not a historical artifact."** This task is specifically about making sure PLAN.md and PRD.md
  catch up to whatever M4 tasks 1–7 actually built, the same discipline every prior milestone's
  close-out already followed (M3's own close-out commit title, "Close out M3 and bump version," is
  the direct precedent this task's own commit should match in shape).

## Out of scope

- **The packaging decision itself** (Homebrew, musl/arm64 Linux, macOS signing, Windows signing) —
  explicitly deferred to the end of alpha, not this close-out, per PRD §10 and the user's
  2026-10-01 scope call recorded in `docs/plans/m4-planning.md`.
- **Any new feature work** — this task only closes out and documents what tasks 1–7 already built;
  if something was found missing during close-out, it is a new task/plan doc, not a change folded
  silently into this one.
- **A 1.0 release** — M4 ends in `0.1.0-beta.1`, explicitly not a 1.0, per PLAN's row and the user's
  scope decision.
