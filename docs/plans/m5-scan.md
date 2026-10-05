# M5 task 3: Merged scan

Status: **done.**

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

## Outcome

Done. A Cluster target now scans every primary and reports completion only when all of them
finished; a topology change ends the scan as interrupted. Implements R1.11 and ADR-0022 (still
Proposed). The refusal gate in `main.rs` is unchanged, so no user sees this until task 5.

- **Shell (`crates/app/src/redis/scan.rs`):** one `stream_keys` for both deployments. Every scan is
  N node scans merged on a small bounded channel into one shared page loop (N = 1 on a standalone
  server, on the caller's own client, with no timeout, so that path behaves as before). The end of
  the scan is the merged channel closing after every node reported `Done`; a page's `has_more()` is
  never consulted, and a node task that vanished is a `ScanFailed`, not a completion. `Esc` is
  answered at the next page boundary and drops the tasks and the per-node connections.
  `stream_keys_with(.., Tuning { page_size, node_page_timeout, topology_sync_every })` is the test
  hook; production uses `Tuning::default()` (500, 15s, 1s).
- **Core:** `Msg::ScanInterrupted { reason: InterruptReason::TopologyChanged }`,
  `ScanState::Interrupted { scanned, reason }`, readout "the cluster changed during the scan — r to
  rescan" (warn token). Keys stay, completion is not recorded, a cap reached first is not
  overwritten, and the existing `r` rescans (no new binding). `ScanBatch`, the cap, sort, filter and
  tree are untouched; the cap is still enforced only in `scan_batch`. Nothing in `terminal.rs` needed
  to learn about interruption: its only scan state is the cancel token, and `Esc` only emits
  `CancelScan` for a *running* scan.
- **Tests:** 4 core unit tests (interrupted state and readout, cap precedence, `r` rescan, no
  completion), 2 golden frames (`browser_interrupted`, `browser_interrupted_60`), and 6 `#[ignore]`d
  integration tests in `cluster_scan`: 30,000 keys seen exactly once and the scan completes;
  progress denominator equals the sum of `DBSIZE` over the primaries; a `user:*` filter across
  nodes; `Esc` mid-scan answered within one page and `SCAN` calls stop on the servers; a failover
  during a held scan yields `ScanInterrupted`, never `ScanComplete`; a dead primary (before and
  during a scan) fails the scan naming the node. Perf suite unchanged against main.

### Deviations

- **Decision 1 fallback taken: not `scan_cluster`.** The confirm-at-build test failed. With one
  primary shut down, `scan_cluster` delivered **no pages at all** (not even from the healthy
  nodes) and **no error** for 40s, using the production client (10s command timeout, 6s
  unresponsive detector): a key scan's errors are not subject to either, so there is nothing to name a
  node with. Cancellation by dropping the stream was not pursued after that. The fallback is
  `split_cluster`: one single-node client per primary (same config, credentials, TLS, perf and
  reconnect policy), one task each, a per-page timeout that names the node.
  A `with_cluster_node(..).scan` per node was tried in the spike and is not safe: its stream ended
  after one or two pages.
- **`DBSIZE` runs on the per-node connections, not `with_cluster_node` on the main client** (decision
  2 said the latter). The main client has one router for all nodes, and a dead primary stalls it, so
  the first `with_cluster_node` call to time out was a *healthy* node and the error named the wrong
  one. Primaries still come from `cached_cluster_state()` (it is what `split_cluster` reads).
  Concurrent, each bounded by the node timeout; any failure fails the scan, never a smaller total.
- **A topology-sync ticker was added** (`sync_cluster()` every 1s for the scan's lifetime). With
  per-node connections the scan no longer drives the main client, and `fred` notices a change only
  when traffic reaches that client, so without it `cluster_change_rx` could stay silent for a whole
  scan on an otherwise idle app. A failed sync is ignored by design (the same trouble fails a node
  scan, which names the node).
- **The readout's "r" is a literal**, not a keymap hint: `ScanState::readout()` has no keymap, as for
  the existing readouts.

### Findings about `fred` 10.1.0 for task 4 (owner-pinned liveness)

- `scan_cluster` is not usable when a node is down (above). A dead primary's commands never error
  on a scan stream; wrap every per-node wait in a timeout.
- `fred`'s main client has a single router: one unreachable node delays commands to the *others*, so a
  timeout on a command does not identify the guilty node. Use a connection per node when blame
  matters.
- `split_cluster()` clients carry credentials, TLS, RESP version and policies, but must be
  `init()`ed and `quit()`ed by the caller; `client_config().server.hosts()` recovers the node.
- `sync_cluster()` on the main client does emit `cluster_change_rx` after a failover (observed within
  the 200ms-1s tick), so a periodic sync is the way to learn of a change without other traffic.
- A demoted primary keeps answering `SCAN` (replicas serve keyless commands), so a per-node scan
  never errors on failover; only the change event reveals it.
