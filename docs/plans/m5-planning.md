# M5 — Cluster: task plan and per-task docs

Status: **in progress — tasks 1–7 done.** The design is [`m5-cluster.md`](m5-cluster.md); this file
splits it into tasks, one Sonnet subagent run each, in the M4 shape (one plan doc per task, built
phase by phase, merged before the next starts).

## Context

M4 closed as `0.1.0-beta.1` (2026-10-05). The M4 analysis (2026-10-03) designed Cluster support
and planned it in four stages. Stage 0, the refusal (ADR-0021, M4 task 1), is shipped. A
2026-10-05 pass checked the design against `fred` 10.1.0 and the code. It found:

- **Routing a command to a slot is supported.** `Options { cluster_hash:
  Some(ClusterHash::Custom(slot)), .. }` via `client.with_options(&opts)` applies to every command
  sent through the wrapper. A `Pipeline` built from that wrapper therefore carries the hash on
  each command, including the keyless `CLIENT CACHING YES`. This is how task 4 pins the arming.
  It is unproven that fred writes a pinned pipeline contiguously on one node connection; task 4's
  first phase is a spike that proves it.
- **`Options.caching` stays off-limits.** `crates/app/src/redis/read.rs` already records it as
  inert in 10.1.0.
- **Tracking:** `start_tracking` with `bcast = false` sends `CLIENT TRACKING` to every node
  (`_ClientTrackingCluster`). That is what `crate::redis::mod.rs` already calls.
  `Invalidation { keys, server }` names the node that sent it.
- **Topology:**
  - `reconnect_rx()` / `on_reconnect` yield the `Server` that reconnected, so task 4 can tell the
    owner from other nodes.
  - `cluster_change_rx()` yields `Vec<ClusterStateChange>` (`Add`, `Remove`, `Rebalance`), the
    signal tasks 3 and 4 need.
  - `cached_cluster_state()` gives the routing table, so the title bar can count primaries and
    nodes.
- **Scan:** `scan_cluster` runs one `SCAN` per primary concurrently. Its stream can continue after
  a page reports `has_more() == false`, so `crates/app/src/redis/scan.rs` must treat "the stream
  ended" as the only end-of-scan signal. `split_cluster` / `with_cluster_node` are the per-node
  alternative.
- **Pub/Sub:** `SubscriberClient` has `ssubscribe`. `feed.rs` drops `MessageKind::SMessage` today.
- **Touch points:**
  - `redis/mod.rs` `build_config` (scheme refusal) and `server_conditions` (`cluster_enabled:1`
    refusal, replica reason);
  - `redis/read.rs`: `read_value` (the arming pipeline), `fetch_server_info` and `fetch_slowlog`;
  - `redis/scan.rs`;
  - `redis/feed.rs` `monitor_config` (its own clustered refusal);
  - `terminal.rs` `watch_link` (the reconnect and invalidation listeners);
  - `core/state/copy.rs` (the `redis-cli` command);
  - `render/mod.rs` `title_bar`.
- **Harness:** the only multi-container test today is the replica test in
  `crates/app/tests/integration.rs` (`host.docker.internal`). The existing
  `start_cluster_enabled()` there is a single-node `--cluster-enabled yes` server, enough for the
  refusal test but not a cluster.

## Scope decisions (user, 2026-10-05)

- **M5 goes first** after the beta, ahead of M6 (rebuild perf) and the unbuilt M2 tasks 11–14.
- **Mutations: guard only what exists.** M5 makes the mutations that ship today correct on a
  Cluster. Every one is single-key, and every guarded script touches only `KEYS[1]`. The Cluster
  rules for rename and copy (a `CROSSSLOT` warning in the preview, a `DUMP`/`RESTORE` fallback for
  copy) and for bulk delete (grouped by node, the typed confirmation counting every node) are
  written into PLAN §5's rows for M2 tasks 11–13. They get built with those features, not here.

## Task table

| # | Task (plan doc) | Proves |
|---|---|---|
| 1 | Cluster test harness + ADR-0022 ([`m5-harness.md`](m5-harness.md)) | A real 3-primary / 3-replica cluster comes up from `testcontainers`, reachable from the test process, locally and in CI. ADR-0022 supersedes ADR-0021 (staged) and amends ADR-0008. No product behaviour changes yet, and the refusal still holds |
| 2 | Connection + title bar ([`m5-connection.md`](m5-connection.md)) | A Cluster target connects behind a gate still closed to users. The title bar reads `cluster · 3 primaries · 6 nodes` beside target and Source. `C` emits `redis-cli -c`. `infer_environment` is unchanged |
| 3 | Merged scan ([`m5-scan.md`](m5-scan.md)) | One cursor per primary feeds the same `Msg::ScanBatch` stream, and the core is untouched. Progress sums `DBSIZE` over primaries. A topology change mid-scan ends in "the cluster changed during the scan — r to rescan", never `Complete`. A 3-primary integration test sees every key exactly once |
| 4 | Owner-pinned liveness ([`m5-liveness.md`](m5-liveness.md)) | The arming and `TYPE` are pinned to the key's slot; an invalidation arrives from the owner. Only the owner's reconnect re-arms. A key that moves, by forced failover or by migration, is re-armed on its new owner. `● live` is never shown over an unarmed key, proven against a real cluster |
| 5 | Open the keyspace, narrow the refusal ([`m5-narrowed-refusal.md`](m5-narrowed-refusal.md)) | The startup refusal is lifted: browsing, the Viewer and liveness work on a Cluster. `g s`/`g m`/`g d` on a Cluster show "per-node on a Cluster — coming in M5" in the view instead of one node's figures. Pub/Sub stays open, since classic `PUBLISH` is cluster-wide |
| 6 | Mutations on a Cluster ([`m5-mutations.md`](m5-mutations.md)) | Every shipped mutation (edit, add, remove, TTL, delete, every guarded script) is integration-tested on a key in each primary's slot range. The `replica` read-only reason applies only when every reachable node is a replica. The Cluster rules for M2 tasks 11–13 are written into PLAN §5 |
| 7 | Dashboard ([`m5-dashboard.md`](m5-dashboard.md)) | Cluster-wide tiles: memory, ops/sec and clients summed, worst replication lag. A `CLUSTER INFO` health line. A node table, one row per node, with alarm colours. `Enter` on a row shows that node's tiles; `Esc` returns. This is a drill-down inside the view, not a connection switch, so ADR-0005 holds |
| 8 | Slowlog + Monitor ([`m5-slowlog-monitor.md`](m5-slowlog-monitor.md)) | One merged Slowlog across nodes with a NODE column; reset confirms "SLOWLOG RESET on N nodes". Monitor runs one feed per primary, merged, with a NODE column; its banner and the `prod`/`unknown` confirm name the node count. `feed.rs`'s clustered refusal is removed |
| 9 | Sharded Pub/Sub ([`m5-sharded-pubsub.md`](m5-sharded-pubsub.md)) | `SSUBSCRIBE` chips, marked sharded and routed to the channel's slot. `SMessage` is shown instead of dropped. Gated on Redis ≥ 7, with a clear message below it |
| 10 | Close out M5 ([`m5-close-out.md`](m5-close-out.md)) | PLAN, PRD, README, ALPHA and ADR-0021/0022 are marked complete; the version is bumped; release notes are drafted |

## Order and gating

1. **Task 1 first.** Every later task proves itself against the cluster it builds. The test-harness
   cost is the reason `m5-cluster.md` calls M5 the largest piece left, so it is paid up front.
2. **Tasks 2 → 3 → 4 in order, all behind the refusal.** Until task 5 ships, a user still sees the
   ADR-0021 diagnostic. Tasks 2–4 reach the cluster only through a test-only path (task 2 decides
   which), so no build ever shows a half-supported Cluster to a user. Task 4 carries the risk:
   liveness is the headline feature, and a wrong arming reopens the RedisInsight bug.
3. **Task 5 is the user-visible switch.** After it, the beta can browse a Cluster with live keys,
   and the server views say plainly that they are not ready yet.
4. **Tasks 6–9 in any order after 5.** 6 is small; 7 is the largest; 8 and 9 are independent.
5. **Task 10 last.**

## How each task runs

Same as M4 (memory: plan execution via Sonnet subagent). Each task:

- starts from synced `main` on its own `m5-<name>` branch;
- runs as one Sonnet subagent working phase by phase from its plan doc, monitored at each
  checkpoint;
- runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and the Docker suite (`cargo test -p redis-pane -- --ignored
  --test-threads=1`);
- ends with a PR that names the requirements it implements, updates the docs in the same change,
  and gets an `## Outcome` section added to its plan doc.

## CLAUDE.md rules M5 binds throughout

- **The keyspace source abstracts over a stream of keys** (`redis/scan.rs`). The core sees only
  `Msg::ScanBatch`, and must not learn about nodes.
- **The Viewer never caches a value**, and **tracking is consumed by its own invalidation**: one
  read path, which re-arms every time (ADR-0006). The two re-arm invariants (after every
  invalidation, after every reconnect, ADR-0009) become *per owner node*.
- **One Connection per process** (ADR-0005). A Cluster is one logical target. The node table is a
  drill-down, never a switcher.
- **Errors surface as notifications carrying the failing command** (R7.4). On a Cluster that
  includes the node.
- **The title bar shows target and Source at all times**, and on a Cluster it also shows the shape.
