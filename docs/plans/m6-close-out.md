# M6 task 7: Close out M6

Status: **done.** Outcome at the end of this document.

## Context

[`m6-planning.md`](m6-planning.md) row 7. This is the same close-out M4 and M5 had: the docs
catch up with what shipped, and the version is bumped in the same change.

## Decisions

1. **PLAN.md:** add a "Progress: done" section for M6. It covers each task, the measured
   before-and-after numbers from task 1's harness, and each task's plan doc. Remove M6 from the
   "still unbuilt" list.
2. **PRD §9:** mark the M6 bullet done. If the shipped figures differ from the goals, PRD §7's
   numbers and M4's qualified 16 ms claim are restated against what was measured.
3. **README and ALPHA:** the "about half a second at a million keys" known limit is replaced by
   what is now true, with the measured figures, and the `rebuilding N%` readout is described.
   The install URLs are re-pinned.
4. **DESIGN.md** documents the rebuilding readout, if task 3 has not already done so.
5. **Version:** the next beta (`0.1.0-beta.N`). The tag goes on the close-out merge commit, and
   it is pushed only after the user confirms.
6. **`m6-perf-rebuild.md` and `m6-planning.md`** are marked done.

## Testing

This task changes only docs and metadata.
- Every version string agrees.
- The install URLs resolve once the release exists.
- The downloaded binary prints the new version.

## Outcome

Built 2026-10-08 after tasks 1–6 merged (#78–#83).

- **PLAN §7.3** (new): progress narrative, per-task table, before/after at 1M deep random keys,
  and the known gaps. The "still unbuilt" line and §8 no longer list M6.
- **PRD §9**: M6 marked done with the measured figures; M4's qualified 16 ms note now says the gap
  was closed by M6.
- **README / ALPHA**: the "half a second at a million keys" limit is replaced by what is true now
  (no update waits on a rebuild; `rebuilding N%`; ~0.1 s to a new list for a tree toggle). ALPHA's
  "What to try" gives the 1M-key recipe, and its known limits list the remaining gaps: a
  synchronous rebuild when restoring a tree-mode session at startup, `Esc` not cancelling a job,
  wildcard/fuzzy filters off the fast path, ASCII-only case folding.
- **DESIGN** already documents the readout (§6.2, task 3).
- **Version** `0.1.0-beta.3`; install URLs re-pinned. Tag after the user confirms.
