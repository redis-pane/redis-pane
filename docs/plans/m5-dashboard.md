# M5 task 7: Cluster Dashboard

Status: **done.** This was the largest of the server-view tasks.

## Context

[`m5-planning.md`](m5-planning.md), row 7. The Dashboard (`g d`, M3, [`m3-dashboard.md`](m3-dashboard.md))
polls `fetch_server_info`, which runs `INFO default` every 2 s, and renders tiles. Those tiles are
memory against `maxmemory`, hit ratio, ops/sec, clients, replication and evictions, and `Enter`
shows the raw `INFO` behind a tile. On a Cluster, task 5 replaced the view with a notice.

ADR-0008 feared that a Cluster Dashboard means a "node selector inside a UI whose entire premise
is that there is nothing to switch". [`m5-cluster.md`](m5-cluster.md) answers that fear with a
drill-down inside the view, not a connection switch.

## Decisions

1. **Fetch:**
   - The shell polls `INFO default` on every node with `with_cluster_node`, concurrently, plus
     one `CLUSTER INFO`.
   - Each node's result is parsed with the existing `parse_info` into a per-node `RawInfo`. The
     core receives `Vec<(NodeId, Role, RawInfo)>` and the cluster info.
   - A node that fails its `INFO` keeps a row marked failed, with the command and the error
     (R7.4). It is not dropped, and the other nodes still render.
2. **The aggregation lives in the core and is pure and unit-tested.**

   | Tile | Value |
   |---|---|
   | Memory | Summed over primaries, against the summed `maxmemory`. If any primary has none, the tile says "no limit on N nodes" |
   | ops/sec | Summed over primaries |
   | Clients | Summed over all nodes |
   | Replication | The worst lag across replicas |
   | Evictions | Summed |
   | Hit ratio | Computed from summed hits and misses, never by averaging the per-node ratios |

3. **Health line:** a line from `CLUSTER INFO` reads `cluster_state ok · 16384/16384 slots ·
   0 failing`. It uses the alarm colour when the state is `fail` or any slot is unassigned.
4. **Node table** below the tiles has one row per node: address, role, slot count, memory %,
   ops/sec and lag.
   - Rows use the same alarm thresholds the tiles use.
   - `j`/`k` move the cursor. `Enter` replaces the tiles with that node's own tiles, using the
     existing single-node rendering, under a breadcrumb `cluster › 10.0.0.3:7001`. `Esc`
     returns.
   - The raw-`INFO` drill-down still works inside a node view.
   - Keys follow the keymap growth rule (DESIGN §4, ADR-0020): they are scoped to the view, with
     no new top-level binding.
5. **Layout budget:**
   - At 80 columns the node table drops its columns in this order: lag, ops/sec, memory %.
   - Below 70 columns it shows address and role only.
   - Height-limited, it scrolls, and the tiles never shrink to fit it.
6. **Non-cluster targets are unchanged:** no node table and no health line. The single-node
   goldens must not move.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/read.rs` | per-node `INFO`, `CLUSTER INFO` |
| `crates/core/src/state/dashboard.rs` | node rows, aggregation, drill-down state |
| `crates/core/src/render/dashboard.rs` | health line, node table, breadcrumb |
| `crates/core/src/update/…`, `keymap` | view-scoped keys |
| `docs/DESIGN.md` (Dashboard screen) | the Cluster layout |

## Testing

- **Unit tests:** every aggregation, including the hit ratio from summed counts, "no limit on N
  nodes", and a failed node.
- **Golden frames:** the Cluster dashboard at 140, 80 and 60 columns; a drilled-in node; a failed
  node row; `cluster_state:fail`.
- **Integration tests:** the 6-node harness produces 6 rows and the health line reads ok. Killing
  one replica's process gives a failed row and the others still render.

## Outcome

Done. `g d` on a Cluster shows the whole Cluster: a `CLUSTER INFO` health line, tiles aggregated
across the nodes, and a node table with one row per node (primaries and replicas). `Enter` opens
one node's own tiles under a `cluster › <addr>` breadcrumb; `Esc` returns. Implements R6.3, R1.11
and ADR-0022. ADR-0005 holds: this is a drill-down inside the view, one Connection, no new
top-level key.

- **Shell (`crates/app/src/redis/cluster_info.rs`, new).** Every poll reads each node on a
  connection of its own, built from the main client's config exactly as `split_cluster` builds
  them (credentials, TLS, RESP3, timeouts, no reconnect policy), and every wait is bounded (4s) and
  names its node. The main client is never sent a command by the poll (only its cached routing table
  is read, a local lookup), so a dead node cannot stall it and the `fred` ASK wedge
  (`liveness::is_wedged`) cannot reach it. `Poller` holds the pool between polls and the poll in
  flight; `Command::CancelServerInfo` (new, emitted only on a Cluster) aborts the poll and closes
  the connections when the view is left by any route, and a reconnect of the main client closes them
  too. A node that fails is evicted from the pool and rebuilt on the next poll. The non-Cluster path
  (`fetch_server_info`) is unchanged. The scan helpers `close` and `describe_scan_error` and the
  `parse_info` parser are shared rather than copied.
- **Node list: `CLUSTER NODES`, not `cached_cluster_state()`.** The routing table lists primaries
  only (replicas only with `fred`'s `replicas` feature and once replicated), so it can never give
  "every node with its role". `CLUSTER NODES` gives every node, its role, its slot ranges, and the
  role of a node that then fails its `INFO`. The routing table's primaries only seed the first
  connections; `CLUSTER NODES` and `CLUSTER INFO` are asked of every seed concurrently and the first
  answer wins, so a dead seed costs nothing. `noaddr` nodes are skipped; an announced hostname
  stands in for an empty host.
- **Core.** `Msg::ClusterInfoLoaded { nodes: Vec<NodeReading>, health, at_ms, token }` (a whole
  poll failing is the existing `ServerInfoFailed`, which keeps the last good nodes on screen with a
  banner). `state/cluster_dash.rs` holds `ClusterDash`: one `DashboardState` per node, keyed by
  address, so the rising-counter alarms and the ops history survive polls, and the Cluster's tiles
  are a pure function of those readings (`ClusterDash::tiles`, unit-tested: every decision-2 rule,
  the hit ratio from summed counts, "no limit on N nodes", a failed node left out). The node view
  reuses the single-node rendering and keys through `DashboardState::active()`, so there is no
  second implementation of the tiles or the raw-`INFO` overlay. A failed node raises one R7.4
  notification (`INFO on <addr>: <error>`), once per failure rather than per poll.
- **Navigation.** `j`/`k` and the arrows move the node cursor, `Enter` opens the node, `r` polls
  now; all are the grid's existing actions, so no new binding exists. `Esc` closes an overlay, then
  returns to the overview, then leaves the view. Help and the hint bar have a `DashboardCluster`
  context ("move node", "open node", "refetch") that reads the keymap like the others. The Dashboard
  is out of the `cluster_notice`/`RefusalReason` path; Slowlog and Monitor stay in it.
- **Layout.** The health line sits under the summary line, then the tiles (grid breakpoints
  unchanged, tiles never shrink), then the table. Columns drop as the width runs out: LAG, OPS/SEC,
  MEMORY; below 70 columns, address and role only. Height-limited, whole tile rows from the bottom
  are left off so the table keeps its header and three nodes; the table windows around the cursor
  and shows `n/N` when it scrolls. One new glyph role, `Crumb` (`›`, ASCII `>`).
- **Tests.** 21 unit tests in `cluster_dash.rs` (aggregation, health, ordering, cursor), 11 in
  `update/dashboard.rs` (`cluster_tests`), 5 in `cluster_info.rs` (`CLUSTER NODES` parsing), 1 in
  `help.rs`; the Cluster `g d` update tests were rewritten. 8 new golden frames: overview at 140, 80,
  60 and ASCII, a drilled-in node, a failed node row, `cluster_state:fail`, and the first-poll
  state. The two old `cluster_dashboard_*` notice goldens were deleted; no single-node golden moved.
  5 `#[ignore]`d integration tests in `cluster_dashboard` on the task 1 harness: 6 rows, health ok
  and every aggregate equal to the sum of the per-node `INFO`s (and summed `maxmemory` after a
  `CONFIG SET`); a killed replica is one failed row naming the node, over the dead connection and
  over a reconnect, with the others rendering and no stall past the per-node timeout; a killed
  primary is the same; cancelling closes the per-node connections (server-side `connected_clients`
  returns to its baseline) and nothing more is sent; a poll cancelled mid-flight never answers.

### Deviations and judgement calls

- **The memory tile is at least as alarming as its worst primary**, not only the summed ratio
  (decision 2 says summed). A sum hides one primary at 99% among two empty ones, which is the node
  that is about to evict or refuse writes. The node table shows the per-node figure either way.
- **A failed node is left out of every aggregate** and counted in the health line (`1 of 6 nodes
  not answering`), rather than summing its last good reading. Its row keeps the last good figures
  inside its own node view, with the stale-error banner.
- **Hit ratio, clients and evictions sum over every node, replicas included;** memory and ops/sec
  over primaries only. Replication lag takes the worst across all nodes (a primary's `slaveN lag=`
  and a replica's `master_last_io_seconds_ago`), and a down link is danger.
- **Decision 5's "at 80 columns" is read as a width-driven drop order**, not a fixed set at exactly
  80: with the column widths used, 14-character addresses at 80 columns lose LAG only, and longer
  addresses lose OPS/SEC as well.
- **Tile rows, not tile height, give way** when the terminal is short (the table is guaranteed a
  header and three rows). On a very short terminal the overview therefore shows fewer tiles; the
  node view has the single-node grid's own scrolling.
- **The cluster overview tiles are not individually focusable** and have no raw-`INFO` overlay (an
  aggregate has no single `INFO` section); that lives in the node view.
- **A poll aborted mid-connect** (view left, or superseded) can drop a half-initialised client
  without `quit()`; bounded and rare, and the pool itself is always closed.
- No reconnect policy was added; the poll uses none on its per-node clients either.
