# M5 task 7: Cluster Dashboard

Status: **planned.** This is the largest of the server-view tasks.

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
