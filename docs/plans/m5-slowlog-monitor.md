# M5 task 8: Slowlog + Monitor on a Cluster

Status: **planned.**

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
