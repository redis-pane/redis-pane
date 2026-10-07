# M5 task 10: Close out M5

Status: **done.** Outcome at the end of this document.

## Context

[`m5-planning.md`](m5-planning.md) row 10. This is the same mechanical close-out as M3 and M4
([`m4-close-out.md`](m4-close-out.md)): the docs catch up with what shipped, and the version is
bumped in the same change.

## Decisions

1. **PLAN.md** gets a "Progress: done" paragraph for M5. It names each task, what shipped, and
   the task's plan doc. "Still unbuilt" drops M5.
2. **PRD §9's M5 bullet** is confirmed against what was built, or updated if the build diverged.
   **R1.11** and the Cluster non-goal read as supported.
3. **ADR-0008's Cluster deferral** is marked as resolved by ADR-0022. ADR-0021 already says
   "superseded" (task 5).
4. **README and ALPHA:**
   - Cluster moves from "not yet" into "What to try": point it at `redis-cluster://`, see the
     shape in the title bar, `g d` for the node table, `g s`/`g m` merged with a NODE column, and
     sharded chips in `g p`.
   - The Known-limits list keeps M6, if it has not shipped.
5. **Version:** bump to the next beta (`0.1.0-beta.N`). The tag goes on the close-out merge
   commit, as for `v0.1.0-beta.1`. **Ask before pushing the tag.**
6. **`m5-cluster.md`'s status** becomes "done", pointing at this file.
7. **Grep for stale wording** across the repo: "refused", "ADR-0021" and "Cluster is planned".
   Nothing user-facing may still say that Cluster is refused.

## Testing

Docs and metadata only. Every version string agrees. The install URLs resolve once the release
exists, and the downloaded binary prints the new version.

## Outcome

Built as planned, on 2026-10-07, after tasks 1–9 merged (#67–#75).

- **PLAN.md:** new §7.2 has M5's "Progress: done" narrative, a per-task table and the `fred`
  10.1.0 findings. The "still unbuilt" line and §8 no longer list Cluster.
- **PRD:** §9's M5 bullet is marked done. R1.11 and the Cluster non-goal describe what shipped:
  the merged scan, owner-pinned liveness, owner-routed mutations, the aggregated Dashboard, the
  merged Slowlog and Monitor, and sharded Pub/Sub.
- **ADR-0008:** marked resolved. ADR-0021 has said superseded since task 5, and ADR-0022 has been
  Accepted since task 5.
- **README and ALPHA:**
  - The README Status describes Cluster as supported.
  - ALPHA's "What to try" has covered Cluster since tasks 5–9, and its stale "Slowlog and Monitor
    not there yet" bullet is removed.
  - The install URLs moved to `v0.1.0-beta.2`.
  - The known limits keep M6 (rebuilds at real `SCAN` order) and the M2 leftovers.
- **`m5-cluster.md` and `m5-planning.md`:** marked done.
- **Version:** `0.1.0-beta.2` in `Cargo.toml`. The tag goes on the close-out merge commit, after
  the user confirms.
- **Stale-wording grep:** I searched README, ALPHA, CLAUDE, CONTEXT, PRD, PLAN, DESIGN and the
  ADRs for "refused", "ADR-0021", "coming in M5" and "planned as M5". The remaining hits are
  history: M4's own §7 narrative, ADR-0021 itself, and ADR-0022's account of superseding it.
