# M5 task 8: Slowlog + Monitor on a Cluster

Status: **done.**

## Context

[`m5-planning.md`](m5-planning.md) row 8. Both commands are per node:

- **Slowlog** (`g s`, [`m3-slowlog.md`](m3-slowlog.md)) uses `fetch_slowlog`, which runs
  `SLOWLOG GET <count>` against one node. `d` resets it.
- **Monitor** (`g m`, [`m3-monitor.md`](m3-monitor.md)) runs on its own feed connection
  (`feed.rs`). `monitor_config` refuses a clustered config outright, proven by
  `a_clustered_config_is_refused_before_ever_reaching_fred`.

Task 5 shows a notice in both views on a Cluster.

## Decisions

### Slowlog

1. **Fetch** `SLOWLOG GET` from every node concurrently, using `with_cluster_node`, primaries and
   replicas alike, because replicas log slow reads too.
2. **Merge into one list with a NODE column.** The sort is unchanged: the existing sort keys,
   with the timestamp first by default. Entry ids are unique only per node, so the identity of a
   row becomes `(node, id)`.
3. **A node that fails** shows a notice line naming the node and the command. The other nodes'
   entries still show.
4. **Reset (`d`):** the confirm dialog reads "SLOWLOG RESET on 6 nodes". It runs on every node.
   If some nodes fail, the result notification names them: "reset on 5 of 6 nodes; 10.0.0.3:7002
   failed: …".

### Monitor

5. **One `MONITOR` feed per primary, merged, with a NODE column.** Replicas are excluded because
   they only replay what their primary already showed. **Confirm at build time** whether fred's
   `monitor::run` accepts a per-node centralized config. If it does not, build one centralized
   `Config` per primary address from the routing table, through the same `build_config` and
   credential path. Rejected: picking a single node, a partial truth presented as the whole
   ([`m5-cluster.md`](m5-cluster.md)).
6. **Cost is stated.** `MONITOR` costs every node it runs on.
   - The banner reads "MONITOR on 3 primaries — costs each of them while open".
   - The `prod`/`unknown` confirm names the count.
   - Leaving the view closes every feed, as today.
7. **Partial failure:** if one primary's feed fails, the others keep streaming, and a line in
   the feed names the node that stopped (R7.4). Pause (`p`) and filter (`/`) apply to the merged
   stream.
8. **Remove `monitor_config`'s clustered refusal** and its test. Replace them with the tests
   below.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/read.rs` | per-node slowlog fetch and reset |
| `crates/app/src/redis/feed.rs` | per-primary Monitor feeds, merged |
| `crates/core/src/state/{slowlog,monitor}.rs`, `render/…` | NODE column, `(node, id)` identity, banners |

## Testing

- **Integration tests:**
  - slow commands forced on two different primaries (`DEBUG SLEEP`, or `CONFIG SET
    slowlog-log-slower-than 0`) both appear, with the correct NODE;
  - reset clears all 6 nodes;
  - Monitor sees a command sent to each primary, tagged with its node;
  - killing one feed leaves the others streaming.
- **Golden frames:** the NODE column at 140, 80 and 60 columns, the Monitor banner on a Cluster,
  and the reset confirm.
- Non-cluster goldens do not move.

## Outcome

Done. `g s` and `g m` on a Cluster show every node, and the task 5 notice path is gone. Implements
R6.1, R6.4, R1.11 and ADR-0022.

- **Shell, shared with the Dashboard (`crates/app/src/redis/cluster_info.rs`).** Node discovery
  (the routing table's primaries as seeds, then `CLUSTER NODES` for every node and its role) is
  split out of `poll` into `discover`, and `slowlog_all` and `reset_slowlog_all` use it with the
  same `NodePool`: one connection per node built from the main client's config, each wait bounded
  (4s) and naming its node, a failed node evicted and rebuilt next time, and no reconnect policy. The
  main client is never sent a command. `Poller` gained `fetch_slowlog` (in the abortable slot the
  poll uses) and `reset_slowlog` (not abortable: leaving the view must not cut a write short; closing
  the pool queues behind it on the pool's lock). `Command::CancelServerInfo` is renamed
  `CloseNodeConnections` and is now also emitted when the Slowlog is left by any route.
- **Slowlog.** `SlowlogEntry.node` (empty off a Cluster), identity `(node, id)`, and
  `Msg::SlowlogLoaded { entries, failed, at_ms }`. Recent is a stable sort by timestamp, which on
  one node is the server's own order and across nodes interleaves them. The selection follows its
  entry by identity across a refetch. A failing node is a line under the summary and one R7.4
  notification (`SLOWLOG GET on <node>: <why>`, plus `(and N more)`). Reset: the confirm reads
  `SLOWLOG RESET on N nodes` and the shell's `Shell::mutate` sends it to every node; a partial
  failure settles as `Err("reset on 5 of 6 nodes; <addr> failed: …")`, which the core reports as
  `SLOWLOG RESET: …` and then refetches to show what is left.
- **Monitor.** `fred::monitor::run` **does** accept a per-node centralized config: it matches on
  `ServerConfig::Centralized` and nothing else, so one feed per primary is the cluster `Config`
  from `build_config` (same credentials, TLS and timeouts) with its server replaced
  (`per_node_config`). Primary addresses come from the main client's cached routing table, a local
  lookup. `monitor_config`'s clustered refusal and its test are gone. Dials are concurrent and
  bounded; a primary that cannot be dialed is a stopped feed from the start, and only no primary at
  all is an error. Each reader tags its lines (`Msg::MonitorLine.node`), a stream that ends is
  `Msg::MonitorNodeStopped`, and the last one to end sends `Msg::FeedClosed`. `FeedHandle` closes
  all the readers. The banner reads `MONITOR on 3 primaries — costs each of them while open`
  (`2 of 3 primaries` after one stops), and the `prod`/`unknown` confirm names the count.
- **Layout (DESIGN §6.7, §6.10).** NODE is never shed and neither is COMMAND. Slowlog on a Cluster:
  from 100 columns AGE, DURATION, NODE, COMMAND, CLIENT; CLIENT yields first (below 100), AGE second
  (below 70), and below 70 NODE narrows from 22 to 16 columns. Monitor: TIME, NODE, CLIENT, COMMAND
  from 100; CLIENT yields first, and below 70 NODE narrows. Non-Cluster layouts and goldens did not
  move.
- **Removed as dead.** `render::cluster_notice`, `help::RefusalReason` (and its `PerNodeOnCluster`
  variant, `Refusal::per_node`, `dim_on_cluster`), the hint bar's filter for it, and the
  `on_cluster()` guards in `update/{slowlog,monitor}.rs`. `Refusal.reason` is now a
  `ReadOnlyReason`.
- **Tests.** Core: 9 new update tests (every Environment for `g s` and `g m`, the confirm for
  `prod`/`unknown`, every route out of the Slowlog closing the connections, the reset through the
  chokepoint and refused under every read-only reason, a partial reset failure, a stopped feed,
  `p` and `/` on a Cluster), column tests for both views, and 12 new golden frames (Slowlog at 140,
  80, 60 and ASCII, a failed node, the reset confirm; Monitor at 140, 80, 60 and ASCII, a stopped
  feed, the `prod` and `unknown` confirms; help for both views, undimmed). Three old
  notice goldens were deleted and the rest regenerated. Shell: one unit test for the per-node config.
  6 `#[ignore]`d integration tests (`cluster_slowlog_monitor`): slow commands forced on two primaries
  (`CONFIG SET slowlog-log-slower-than 0` on all six) both appear with the right NODE and all six
  nodes answer; the reset through `Poller` clears all 6; a dead node is named in the fetch and in a
  partial reset (`reset on 5 of 6 nodes`) while the others still work; Monitor sees a command sent to
  each primary tagged with its node and no replica is a monitor client; `CLIENT KILL` of one primary's
  feed leaves the others streaming and names the stopped node without closing the view; leaving the
  view closes every feed (`CLIENT LIST` shows none).

### Deviations and judgement calls

- **The stopped-node "line in the feed" is a header line, not a row in the tail.** It sits under the
  status line, so it survives scrolling and the 5,000-line cap, and is two lines at most.
- **DB is dropped from the Monitor columns on a Cluster** (a Cluster has only database 0).
- **The reset confirm's node count is `Topology.nodes`** (the routing table with replicas); the
  shell resets every node `CLUSTER NODES` lists and the settled message counts those. They agree on a
  healthy Cluster.
- **`r` on Monitor still reopens only when every feed has closed.** One stopped feed among live ones
  is not reopened by `r`; leave and re-enter (`g m`) to redial them all.
- **A failed reset refetches** (on a Cluster only), because a partial reset has still emptied some
  nodes.
- **The Slowlog's selection now follows its entry across a refetch** (by `(node, id)`) on every
  target, not only a Cluster.
- Monitor's feed dial has no reconnect policy and `fred::monitor::run` still hands back no handle,
  so each socket closes when its forwarding task next has a line to send (the single-node limitation
  recorded in `feed.rs`); the integration test nudges each primary, as the single-node one does.
