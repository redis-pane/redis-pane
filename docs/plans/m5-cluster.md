# M5 — Cluster

Status: **planned — not started.** The task breakdown is [`m5-planning.md`](m5-planning.md). Scheduled after the beta (`0.1.0-beta.1`, the end of M4).
Decided 2026-10-03: Cluster is its own milestone rather than part of M4. Until M5 ships, a Cluster
target is refused at startup ([ADR-0021](../adr/0021-cluster-refused-until-supported.md), M4
task 1, [`m4-cluster-refusal.md`](m4-cluster-refusal.md)). That refusal is "stage 0" of this
plan. Building M5 will need an ADR of its own superseding ADR-0021's refusal and amending
[ADR-0008](../adr/0008-sentinel-in-v1-cluster-deferred.md).

## Why Cluster at all

Production Redis at scale is very often clustered: ElastiCache with cluster mode enabled,
Memorystore Cluster, Redis OSS Cluster self-hosted. The on-call engineer on a bastion host, who is
the user this project exists for, is exactly the one who meets a Cluster. RedisInsight supports
Cluster, so without it redis-pane cannot replace RedisInsight for those teams. The architecture
already prepared for it: the keyspace source abstracts over a *stream of keys* so that "N cursors
can replace one without a rewrite" (CLAUDE.md, ADR-0008).

## What a Cluster changes for a client

| Fact | Consequence |
|---|---|
| The keyspace is split into 16,384 hash slots; each key belongs to one primary | Per-key reads and writes go to the owner — `fred` already routes by key |
| `SCAN`, `DBSIZE`, `INFO`, `SLOWLOG`, `MONITOR` are per node | Every "whole server" view asks every node and combines the answers |
| `CLIENT TRACKING` is per connection, per node | Liveness must be armed on the node that owns the open key |
| Database 0 only; multi-key commands must stay in one slot | No `SELECT`; `RENAME`/`COPY` across slots fail with `CROSSSLOT` |
| Topology changes during a session (failover, resharding) | A key can move to another node; scans and tracking must notice |

## Design by feature

**Connection and title bar.** `fred` discovers the topology from a `redis-cluster://` URL. The
title bar reads `cluster · 3 primaries · 6 nodes` alongside the target and Source, so the
"always show what you are connected to" rule holds. ADR-0005 holds too: a Cluster is one logical
target, chosen at launch, with nothing to switch. The `C` copy-command action adds `-c` so the
`redis-cli` it produces follows `MOVED` redirects. `infer_environment` treats `-cluster` schemes
like their plain counterparts (M4 task 1 already does this).

**Keyspace browser.** One `SCAN` cursor per primary, merged into the same `Msg::ScanBatch`
stream. The core is unchanged: list, sort, filter, tree, the Loaded-set cap (ADR-0010) and the
memory model all work as before. Progress sums `DBSIZE` over primaries. On a topology change
during a scan (Redis itself may then miss or repeat keys), the scan does not claim `Complete`; it
says "the cluster changed during the scan — r to rescan". `fred`'s `scan_cluster` stream can keep
producing pages after one page reports `has_more() == false`, so the shell's end-of-scan
condition must change (ADR-0021).

**Viewer and liveness** (the delicate part). Today `CLIENT CACHING YES` is keyless and lands on an
arbitrary node while `TYPE` goes to the owner, so `● live` can be shown over an unarmed key. In
M5:
- The arming and its first read are sent together, **pinned to the open key's slot**, so both
  land on the owner's connection; invalidations then arrive from that node.
- Reconnects are per node. Only a reconnect of the **owner** re-arms; an unrelated node's
  reconnect must not refetch.
- If the key moves (resharding or failover), tracking on the old node is gone: the app re-arms on
  the new owner. ADR-0006's guarantee — never `● live` over an unarmed key — must hold in every
  case, proven by tests against a real cluster, including a forced failover.

**Mutations.** Every existing mutation is single-key, and every guarded script touches only
`KEYS[1]`, so all of them already work on a Cluster. For the M2 leftovers:
- Rename/copy to a key in another slot: the preview warns before running (`CROSSSLOT`); copy may
  fall back to `DUMP`/`RESTORE`, still behind the same confirm dialog.
- Bulk delete: grouped by node; the typed confirmation on `prod` still counts every key across
  all nodes.
- Move-across-database: does not exist in a Cluster; hidden.

**Dashboard.** ADR-0008 feared a "node selector inside a UI whose entire premise is that there is
nothing to switch". The answer is a drill-down inside the view, not a connection switcher:
- Default: cluster-wide figures — memory and ops/sec summed, clients summed, worst replication lag
  — plus a health line from `CLUSTER INFO` (state ok/fail, slots assigned, nodes failing).
- A node table below: one row per node (role, slot count, memory %, ops/sec, lag), alarms
  colouring the row.
- `Enter` on a node shows that node's tiles; `Esc` returns.

**Slowlog.** One merged list across nodes with a NODE column. Reset clears every node; its confirm
dialog reads "SLOWLOG RESET on 3 nodes".

**Monitor.** `MONITOR` is per node. Recommended: one feed per primary, merged, with a NODE column.
Its cost multiplies by N, so the banner and the `prod`/`unknown` confirm name the node count. A
single-node pick is the alternative, rejected as a partial truth presented as the whole.

**Pub/Sub.** Classic `PUBLISH` is broadcast to every node, so ordinary subscriptions already see
cluster-wide traffic. Added: sharded Pub/Sub (`SSUBSCRIBE`, Redis 7+), with chips marked sharded
and routed to the channel's slot; `SMessage`s that `feed.rs` drops today are shown.

**Read-only Mode.** Writes only go to primaries in a Cluster, so the `replica` reason applies only
when every reachable node is a replica.

## Testing

The real cost of M5. It needs a multi-node Cluster in CI: several containers joined by
`redis-cli --cluster create`, or a ready-made cluster image. Nodes must announce addresses the
test process can reach, which is fiddly in Docker (`cluster-announce-ip`/`-port`, or host
networking). Today's only multi-container pattern is the `host.docker.internal` replica test in
`crates/app/tests/integration.rs`. Required scenarios: merged scan completeness; tracking armed on
the owner and an invalidation delivered; per-node reconnect re-arming only the owner; forced
failover and key migration re-arming the new owner; `CROSSSLOT` guards; aggregated `INFO` and
merged `SLOWLOG`.

## Pros and cons

**For:**
- It is where the target user works; without it redis-pane cannot replace RedisInsight for
  clustered teams.
- The key-stream abstraction was built for exactly this.
- Most features adapt additively (a merge, a NODE column) rather than being redesigned.

**Against, or risks:**
- The largest single piece left in the roadmap — likely bigger than Monitor, Pub/Sub and the
  Dashboard combined, mostly in tests and edge cases.
- Liveness is the headline feature, and Cluster makes it harder to keep correct.
- Topology changes mid-session add failure modes a single server never has.
- Node detail leaks into the UI (NODE columns, the node table), contained to the server views.

## Stages (M5's task list)

| # | Stage | Delivers | Size |
|---|---|---|---|
| 0 | Refusal | Done in M4 task 1 — the safe state until this ships | small |
| 1 | Test harness | A real 3-primary cluster in testcontainers; the title-bar readout | medium |
| 2 | Keyspace | Merged scan, owner-pinned tracking, per-node reconnect, `redis-cli -c` | medium-large |
| 3 | Mutations | `CROSSSLOT` guard, grouped bulk delete (with M2's rename/copy/bulk) | small |
| 4 | Server views | Aggregated Dashboard with a node table, merged Slowlog and Monitor, sharded Pub/Sub | large |

Once stage 2 ships, the refusal narrows from the whole target to the server views ("per-node on a
Cluster — coming in stage 4"), so a beta-era user can browse a Cluster before the views are done.
