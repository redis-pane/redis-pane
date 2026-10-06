# M5 task 5: Open the keyspace, narrow the refusal

Status: **done.**

## Context

[`m5-planning.md`](m5-planning.md), row 5. After tasks 2–4, the keyspace and the Viewer work
correctly on a Cluster, but only behind task 2's `ClusterSupport::Refuse` gate. This task makes
them available to users. The server views are not ready yet, and they must not silently show one
node:

- **Dashboard** (`fetch_server_info`) and **Slowlog** (`fetch_slowlog`) would read whichever node
  answered.
- **Monitor** already refuses through `feed.rs` `monitor_config`.
- **Pub/Sub** with classic `SUBSCRIBE` is genuinely cluster-wide, because `PUBLISH` is
  broadcast.

[`m5-cluster.md`](m5-cluster.md) records this as the stage-2 promise: "the refusal narrows from
the whole target to the server views".

## Decisions

1. **`main.rs` passes `ClusterSupport::Allow`.** If nothing else uses `Refuse`, delete it, and
   with it the ADR-0021 startup diagnostic. The `-cluster` scheme handling in
   `infer_environment` stays.
2. **Each unready server view renders an in-view notice instead of data** on a Cluster:
   "Slowlog is per node on a Cluster — coming in M5 (task 8)." The notice uses the view's own
   wording and is not a modal. `g s`, `g m` and `g d` still navigate, and `Esc` and `g k` return.
   No fetch is issued, so nothing reads a single node.
   - The check is core state (`Connection::topology.is_some()`), so goldens cover it.
   - Monitor's existing `monitor_config` refusal stays as a second guard until task 8.
3. **Pub/Sub stays open.** Classic subscriptions are cluster-wide, and sharded ones are task 9.
4. **ADR-0022 becomes Accepted, and ADR-0021 becomes "Superseded by ADR-0022".** CLAUDE.md, PRD
   (R1.11 and the non-goal), PLAN §8, README and ALPHA move from "Cluster is refused" to "Cluster
   browsing works; the server views come later in M5", in this same change.
5. **A release is optional.** If the user wants testers on a Cluster early, cut `0.1.0-beta.2`
   here. **Ask at merge time.**

## Files touched

| File | Change |
|---|---|
| `crates/app/src/main.rs`, `redis/mod.rs` | gate opened; `Refuse` removed if unused |
| `crates/core/src/render/{slowlog,dashboard,monitor}.rs` (or wherever each view renders) | the per-node notice |
| `docs/adr/0021-…`, `docs/adr/0022-…` | status lines |
| `CLAUDE.md`, `docs/PRD.md`, `docs/PLAN.md`, `README.md`, `ALPHA.md` | Cluster wording |

## Testing

- **Golden frames:** each of the three views on a Cluster, showing the notice.
- **Integration tests:**
  - a `redis-cluster://` launch path (`connect_with` with `Allow`) browses every primary's keys;
  - the old refusal tests are rewritten to assert *success*, or deleted if `Refuse` is gone;
  - the per-node Viewer tests from task 4 now run through the production gate.
- **By hand:** `./scripts/` does not start a cluster. Use the task 1 harness through a test, or a
  short note in `scripts/README.md` if a manual cluster recipe proves useful.

## CLAUDE.md rules this binds

- Errors surface; never swallow one silently. A server view that would show one node's figures
  as the whole is the same defect.
- When behaviour diverges from the docs, update the docs in the same change.

## Out of scope

The server views themselves (tasks 7–9).

## Outcome

Done. A real user can browse a Cluster. Implements R1.11, ADR-0022 (now Accepted) and supersedes
ADR-0021.

- **Gate opened by deletion.** Nothing needed `Refuse`, so `ClusterSupport` itself (a one-variant
  enum would be dead weight), the `support` parameter of `build_config`/`connect_with`/`open_feed`,
  `Reconnect.cluster`, and `ConnectError::Cluster` with its ADR-0021 diagnostic are gone. A plain
  `redis://` to a cluster node always redials as a cluster client; if that redial fails the error is
  `Unreachable("<url> is a Redis Cluster node, but dialling it as a Cluster failed: ...")`.
  `infer_environment`'s `-cluster` handling stays. `--print-target` and `--probe` work on a cluster
  URL; `--probe` gained a `cluster: N primaries, M nodes` line.
- **Server views.** `State::on_cluster()`; `open_slowlog`/`open_dashboard`/`open_monitor_view`
  emit no `FetchSlowlog`/`FetchServerInfo`/`OpenFeed` on a Cluster (Monitor skips its prod cost
  dialog too), the poll tick is inert, and every view-scoped action (`r`, `d` reset, `/`, `p`, ...)
  is a no-op, so `SLOWLOG RESET` cannot be staged. `render::cluster_notice` draws the in-view
  notice in each view's wording. Help dims every row of the three views with "per node on a
  Cluster" (`RefusalReason::PerNodeOnCluster`); the hint bar omits them. Pub/Sub untouched.
  Monitor's `monitor_config` refusal stays as a second guard (message reworded).
- **Tests.** 5 core update tests (`cluster_server_view_tests`, all four Environments), 8 new golden
  frames (3 views x 140/80, one ASCII, one help overlay). Integration: the refusal tests were
  rewritten (a cluster-enabled node with no slots now fails visibly naming "cluster") or deleted;
  new `the_production_connect_path_browses_every_primary_by_either_url` and
  `pubsub_feed_opens_on_a_cluster_and_receives_a_publish`. The task 4 liveness tests already used
  `connect_with`, which is now the production path.
- **Docs.** ADR-0022 Accepted, ADR-0021 Superseded, ADR-0008, CLAUDE.md, PRD, PLAN, DESIGN, README,
  ALPHA, `scripts/README.md` (a verified local-cluster recipe; host port 7000 is taken by AirPlay on
  macOS, so 7100-7105).
- **Release.** Cutting `0.1.0-beta.2` is optional: ask the user at merge time.
