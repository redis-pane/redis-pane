# M6 task 7: Close out M6

Status: **planned.**

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
