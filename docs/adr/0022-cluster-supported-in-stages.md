# ADR-0022 — Cluster is supported, in stages

**Status:** Accepted (M5 task 5) · **Date:** 2026-10-05

Accepted in M5 task 5, the change that lets a user reach a Cluster. It supersedes
[ADR-0021](0021-cluster-refused-until-supported.md) from that same change, whose status line reads
"Superseded by ADR-0022 (from M5 task 5)". The `ClusterSupport` switch and the `ConnectError::Cluster`
diagnostic that implemented the refusal were deleted with it.

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
3. **Tasks 7-9 open the server views** (tasks 7 and 8, the Dashboard, Slowlog and Monitor, are
   done; each reads every node on a connection of its own, see `docs/plans/m5-dashboard.md` and
   `docs/plans/m5-slowlog-monitor.md`). The Dashboard (cluster-wide tiles plus a node table),
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

**Proven by task 4's spike, against a real 3-primary cluster** (`cluster_liveness` in
`crates/app/tests/integration.rs`). The pipeline is built as `Pipeline::from(client.with_options(..))`,
because `client.with_options(..).pipeline()` derefs to the plain `Client` and would drop the pin.
For a key on each primary, in loops of re-arms, quiet and under four tasks of unrelated metadata
reads and `GET`s to every primary on the same client:

- every arming was followed by an `Invalidation` whose `server` is the owner, never another node;
- `CLIENT TRACKINGINFO` is observable: it lists `caching-yes` while a `CLIENT CACHING YES` waits
  for its command, and `with_cluster_node(non_owner)` runs it on this client's own connection to
  that node. After each arming no non-owner showed `caching-yes`, and each showed `optin`;
- the **negative control** discriminates: the same pipeline without the pin lost 10 of 15
  armings (every key whose owner is not the node `fred` picks for a keyless command), so the
  tests above would have failed on a mis-routed or interleaved arming.

So `fred` writes a pinned pipeline contiguously on the owner's connection. What the spike found
beyond that, all handled in task 4:

- **A stale routing table mis-arms silently.** If the table names the old owner after a
  failover or migration, `CLIENT CACHING YES` (keyless, never redirected) is spent on the old
  node while `TYPE` follows its `MOVED` to the new one, and the pipeline still succeeds. `fred`
  corrects its table as it follows the redirect, so `read_value` compares the slot's owner before
  and after the pipeline and re-sends the arming when it changed (3 attempts, then an error).
- **`fred` never reconnects a dropped node on its own without a reconnect policy, and the
  client is unusable until it is redialled.** A `CLIENT KILL` on one node's connection gave
  `error_rx` an `IO` error naming the node, no `reconnect_rx`, and from then on every command to any
  node timed out, `sync_cluster` included. So on the app's Cluster client (which has no policy,
  as before) *any* node's dropped connection is a lost link and the shell redials, refetching and
  re-arming the open key; the plan's "an unrelated node's reconnect does not refetch" cannot happen
  there. A policy would let one node heal in place, and it was tried: with a node down, `fred`
  10.1.0's router retries buffered commands in a loop that never yields, which starved the test
  runtime (task 3's dead-primary scan test hung) and would burn a core in the app. It was
  rejected. The owner filter in `liveness::watch_reconnects` is kept for the day a client does
  reconnect in place, and is tested against a test-only client that has a policy.
- **A reconnect on a node that does not hold the arming still needs `CLIENT TRACKING ON`**, since
  `fred` does not re-send it. That filter re-probes tracking and sends nothing else.
- **`fred` 10.1.0 wedges permanently on a read of a key whose slot is mid-migration (`ASK`).**
  The read fails with `Routing`, "Max attempts reached", and every later command on that client,
  `sync_cluster` included, times out, even after the migration ends. Reproduced with a plain
  `GET` and with the app's pinning removed; fresh clients are unaffected. A read that fails with
  `Routing` or `Timeout` on a Cluster is therefore also reported as a lost link, so the redial
  replaces the client.
- **A failover is noticed late**, as in task 3: the shell asks the client to `sync_cluster()` once
  a second while a key is armed, so `cluster_change_rx` fires without other traffic.

ADR-0006's guarantee, that `● live` is never shown over a key no node is tracking for this
client, is the acceptance test, not an aspiration. It holds in tests after an invalidation, after
the owner reconnects, after an unrelated node reconnects, after a forced failover and after a
slot migration. One residual, accepted: when the arming was spent on a stale owner, that node's
connection keeps a pending opt-in until its next command, which can track one unrelated key and
cause a single spurious refetch.

### Mutations and the `replica` reason on a Cluster

Every shipped mutation is single-key and every guarded script touches only `KEYS[1]`, so `fred`
routes each to the slot's owner and none can raise `CROSSSLOT` (audited in task 6, and proven per
primary by the integration suite). A write through the main client inherits the `ASK` wedge above:
the mutation path (`mutate::execute_settled`) reports it as the failed command *and* as a lost
link, so the shell redials.

`Replica` read-only is a property of the whole Cluster, not of whichever node answered `INFO`. It
applies only when every reachable node in `CLUSTER NODES` is a replica (`cluster_is_all_replica`);
a Cluster seeded through a replica is not locked. The routing table (`cached_cluster_state`) cannot
express "no primary", since its entries are primaries by construction, so `CLUSTER NODES` is the
source. Nodes flagged `fail`, `fail?`, `handshake` or `noaddr`, or with a down link, do not count.

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

- ADR-0021 is superseded from M5 task 5. The refusal and its diagnostic held exactly as ADR-0021
  specifies until then, and were removed in that change.
- ADR-0008's "Amended by" gains this ADR: Cluster is now planned and staged, not deferred.
- ADR-0005 holds. A Cluster is one logical target; the Dashboard's node table is a drill-down
  inside a view, never a connection switcher.
- The Docker integration suite gains a cluster that takes several seconds to form. Tests that need
  one start their own, as the suite already does for every container.
- Task 4's spike proved contiguous pinned pipelines, so the arming section stands as decided.
- **Server views (tasks 7 and 8).** The three views that read one node's own figures ask every
  node over `crates/app/src/redis/cluster_info.rs`: one connection per node built from the main
  client's config, never the main client (whose one router a dead node stalls), each wait bounded
  and naming its node, and the pool closed on leaving the view and on a reconnect. Slowlog asks
  every node (primaries and replicas) and merges with a NODE column; its reset runs on every node
  through the mutation chokepoint. Monitor opens one `MONITOR` feed per primary: `fred::monitor::run`
  takes `ServerConfig::Centralized` only, so each feed is the `build_config` result with its server
  replaced by one primary (same credentials and TLS). One feed failing stops that node's feed, never
  the view. The in-view "per node on a Cluster" notice and its dimmed help rows are gone.

## Amends

[ADR-0008](0008-sentinel-in-v1-cluster-deferred.md).

## Sources

- [`plans/m5-cluster.md`](../plans/m5-cluster.md), [`plans/m5-planning.md`](../plans/m5-planning.md),
  [`plans/m5-harness.md`](../plans/m5-harness.md), [`plans/m5-liveness.md`](../plans/m5-liveness.md).
- [ADR-0006](0006-liveness-without-a-refresh-button.md),
  [ADR-0009](0009-connection-lifecycle.md), [ADR-0021](0021-cluster-refused-until-supported.md).
- `crates/app/tests/support/cluster.rs` — the harness.
