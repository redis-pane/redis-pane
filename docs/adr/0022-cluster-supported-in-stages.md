# ADR-0022 — Cluster is supported, in stages

**Status:** Proposed · **Date:** 2026-10-05

Becomes **Accepted** in M5 task 5, the moment a user can reach a Cluster. It supersedes
[ADR-0021](0021-cluster-refused-until-supported.md) from that same change, which is also where
ADR-0021's status line gains "Superseded by ADR-0022 (from M5 task 5)". Until then ADR-0021 is in
force and the refusal is unchanged.

## Context

[ADR-0008](0008-sentinel-in-v1-cluster-deferred.md) deferred Cluster and named what real support
must solve. [ADR-0021](0021-cluster-refused-until-supported.md) made the deferral honest: a Cluster
target is refused at startup, because connecting it silently answered a smaller question than the
one asked (one node's `SCAN`, one node's `INFO`, and a `CLIENT CACHING YES` armed on a different
node from the one the value is read from). It named M5 as the milestone that supersedes it.

M5 is that milestone ([`plans/m5-planning.md`](../plans/m5-planning.md)). It is too large to open
in one change, and the refusal is the only thing standing between a user and the defects above, so
it cannot be lifted until the things it guards against are each proven fixed. The staging below is
how the refusal stays true while the work lands.

## Decision

**Cluster is supported, and it opens in stages. The ADR-0021 refusal holds until the stage that
makes browsing safe, and is lifted exactly there.**

1. **Tasks 2-4 build behind the refusal.** The Connection and title bar (task 2), the merged scan
   (task 3) and owner-pinned liveness (task 4) are reachable only through test-only paths. No
   build shows a half-supported Cluster to a user.
2. **Task 5 lifts the refusal for the keyspace.** Browsing, the Viewer and liveness work on a
   Cluster. Dashboard, Slowlog and Monitor say plainly, inside the view, that they are per-node on
   a Cluster and not yet available, instead of showing one node's figures. Pub/Sub stays open,
   since classic `PUBLISH` is cluster-wide. This is the change that accepts this ADR.
3. **Tasks 7-9 open the server views.** The Dashboard (cluster-wide tiles plus a node table),
   Slowlog and Monitor (merged, with a NODE column) and sharded Pub/Sub each replace their
   "per-node" notice with the real view. Task 6 proves every shipped mutation on a Cluster.

Two design decisions are recorded here because everything above depends on them.

### Liveness is pinned to the key's owner

The arming pipeline (`CLIENT CACHING YES` plus `TYPE`) is built from a client wrapper carrying
`ClusterHash::Custom(slot)` for the open key, so both commands go to the owner's connection and
nothing can interleave between them. The shell records the owner `Server` beside the open key and
refreshes it on every re-arm; the core gains no node type. Consequences, all of which preserve
ADR-0006 and the two re-arm invariants of ADR-0009 *per owner node*:

- an invalidation is attributed to the owner (`Invalidation::server`);
- only the owner's reconnect re-arms; another node's reconnect is ignored for liveness;
- a key that moves (failover, slot migration, seen via `cluster_change_rx`) is treated as a
  reconnect: `● live` drops, then the key is refetched and re-armed on its new owner;
- `Options.caching` stays unused (inert in `fred` 10.1.0).

This is **unproven until task 4's first phase**: a spike must show `fred` writes a pinned pipeline
contiguously on one node connection. If it does not, task 4 stops and this section is amended with
the chosen alternative (`with_cluster_node` plus a per-node lock, or a dedicated per-node
connection). ADR-0006's guarantee, that `● live` is never shown over a key no node is tracking for
this client, is the acceptance test, not an aspiration.

### The test harness uses identity port mapping

A Cluster node announces an address that the client then dials (`MOVED`, `CLUSTER SLOTS`), so every
announced address must be reachable from the test process. Container IPs are not routable from the
host on Docker Desktop for macOS. The harness
(`crates/app/tests/support/cluster.rs`) therefore:

- picks free host ports at test time, from 20000-31999 (below the range Docker uses for its own
  random host mappings, see the plan's Outcome for why), and starts **one** `redis:8.4-alpine` container running
  **six** `redis-server` processes (3 primaries, 3 replicas) on exactly those ports, each mapped
  `host:N -> container:N` with `cluster-announce-ip 127.0.0.1`, using `testcontainers`'
  `with_mapped_port`;
- gives each node an explicit `cluster-port` (also a free port, never mapped), because the default
  bus port `N + 10000` overflows 65535 for an ephemeral `N`;
- forms the cluster with `redis-cli --cluster create ... --cluster-replicas 1 --cluster-yes` run
  inside the container, and is ready only when every node reports `cluster_state:ok` and the
  3-primary / 3-replica shape, polled against a deadline, never a sleep;
- exposes `key_in_slot_of`, `failover` (`CLUSTER FAILOVER` on a replica) and `migrate_slot`
  (`SETSLOT IMPORTING/MIGRATING`, `MIGRATE`, `SETSLOT NODE`) so later tasks do not each rebuild them.

## Alternatives considered

- **Lift the refusal as soon as the scan merges (task 3).** Rejected: the scan is only one of the
  silent-wrong-answer paths. Until liveness is owner-pinned and proven, a Cluster Viewer can still
  claim `● live` over an unarmed key, which is the defect ADR-0021 exists to prevent.
- **Open everything at once, in one release.** Rejected: the Dashboard alone is the largest piece
  left in the project, and holding browsing back for it keeps the most-requested capability behind
  the least-critical one.
- **Keep the refusal, never support Cluster.** Rejected: Cluster users are real, and the
  milestone's premise (ADR-0008) was always when, not whether.
- **One container per node for the harness.** Rejected: announce addresses would have to be
  reachable both between containers and from the host, which needs host networking, and that does
  not work on macOS.
- **A third-party cluster image.** Rejected: unpinned and unaudited, and it hides the
  configuration the tests depend on.

## Consequences

- ADR-0021 is superseded from M5 task 5, not before. Until then the refusal and its diagnostic are
  exactly as ADR-0021 specifies.
- ADR-0008's "Amended by" gains this ADR: Cluster is now planned and staged, not deferred.
- ADR-0005 holds. A Cluster is one logical target; the Dashboard's node table is a drill-down
  inside a view, never a connection switcher.
- The Docker integration suite gains a cluster that takes several seconds to form. Tests that need
  one start their own, as the suite already does for every container.
- If task 4's spike disproves contiguous pinned pipelines, the arming section of this ADR is
  rewritten before task 5 can accept it.

## Amends

[ADR-0008](0008-sentinel-in-v1-cluster-deferred.md).

## Sources

- [`plans/m5-cluster.md`](../plans/m5-cluster.md), [`plans/m5-planning.md`](../plans/m5-planning.md),
  [`plans/m5-harness.md`](../plans/m5-harness.md), [`plans/m5-liveness.md`](../plans/m5-liveness.md).
- [ADR-0006](0006-liveness-without-a-refresh-button.md),
  [ADR-0009](0009-connection-lifecycle.md), [ADR-0021](0021-cluster-refused-until-supported.md).
- `crates/app/tests/support/cluster.rs` — the harness.
