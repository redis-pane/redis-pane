# M5 task 3: Merged scan

Status: **planned.**

## Context

[`m5-planning.md`](m5-planning.md), row 3. `crates/app/src/redis/scan.rs` drives one cursor in
three steps:

1. `client.dbsize()` gives the `estimated_total`.
2. `client.scan(pattern, Some(PAGE), None)` produces pages.
3. When the stream ends, the shell sends `Msg::ScanComplete`.

On a cluster client, `dbsize` answers for one node and `scan` walks one node, so the app finishes
early and reports the scan as complete when it is not. That is the bug ADR-0021 recorded.
CLAUDE.md built the keyspace source as "a stream of keys, not a cursor" so that N cursors could
replace one. This task cashes that in.

`fred`'s `scan_cluster` scans every primary concurrently. Its documentation warns that:

- the stream can continue after a page reports `has_more() == false`;
- a topology change during the scan can miss or repeat keys.

## Decisions

1. **Use `scan_cluster` on a Cluster and `scan` otherwise, behind one function.**
   - The page loop, cancellation (`Esc` at the next page boundary) and `Msg::ScanBatch` are
     shared.
   - "The stream returned `None`" is the only end-of-scan signal on both paths. A page's
     `has_more()` is never consulted for the end.

   The alternative was hand-rolled `split_cluster` with one task per node merged through a
   channel. It is kept as the fallback if `scan_cluster` turns out to be unable to report a
   per-node error or to cancel promptly. **Confirm at build time** by testing cancellation and a
   failing node.
2. **Progress sums `DBSIZE` over primaries.** `DBSIZE` runs on each primary through
   `with_cluster_node`, using the primaries from `cached_cluster_state()`. A failing primary makes
   the scan fail with the node named, not a smaller total.
3. **A topology change during a scan ends it honestly.** The shell watches `cluster_change_rx()`
   for the scan's lifetime. On any change it stops and sends a new
   `Msg::ScanInterrupted { reason: TopologyChanged }`. The core shows "the cluster changed during
   the scan — r to rescan" and does not record completion. Keys already loaded stay, as they do
   after `Esc`.
4. **No dedup in the shell.** A repeat key during a stable scan cannot happen, because each
   primary owns disjoint slots. During a topology change the scan is already declared
   interrupted. The core's Loaded set is the only place keys are stored, and the cap stays
   enforced in `scan_batch` alone (ADR-0010).
5. **The core does not learn about nodes.** `ScanBatch`, the cap, sort, filter and tree are
   untouched. The only core change is the new interrupted state and its readout.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/scan.rs` | cluster branch, summed `DBSIZE`, topology watch |
| `crates/core/src/msg.rs`, `update/scan.rs` | `ScanInterrupted`; readout state |
| `crates/core/src/render/…` | the interrupted readout (golden frame) |

## Testing

- **Integration, 3 primaries:**
  - 30,000 keys spread over every slot range are seen exactly once each, and the scan completes;
  - the progress denominator equals the cluster-wide total;
  - `Esc` mid-scan cancels within one page;
  - a pattern filter (`user:*`) works across nodes.
- **Integration, topology change:** a `CLUSTER FAILOVER` (task 1 helper) during a slow scan
  (small `PAGE` via a test hook, or a large keyspace) produces `ScanInterrupted`, never
  `ScanComplete`.
- **Unit tests:** the core's interrupted state, its readout, and `r` rescanning from it.
- **Perf:** the core perf suite is unchanged (`--test-threads=1`), and its numbers must not move.

## CLAUDE.md rules this binds

- `SCAN` only, never `KEYS`. The scan is cursor-based, streaming and cancellable.
- The keyspace source abstracts over a stream of keys, and the core sees only `Msg::ScanBatch`.
- The cap is enforced in exactly one place.

## Out of scope

The core's rebuild cost at real `SCAN` order (M6). A merged scan is still effectively random
order, so M6's finding applies unchanged.
