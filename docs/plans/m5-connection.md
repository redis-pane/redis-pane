# M5 task 2: Connection + title bar

Status: **done.**

## Context

[`m5-planning.md`](m5-planning.md), row 2. Today the app refuses a Cluster in two places, both in
`crates/app/src/redis/mod.rs`:

- `build_config` returns `ConnectError::Cluster` when `config.server.is_clustered()`. That is the
  `redis-cluster://` scheme, in every spelling `fred` accepts.
- `server_conditions` refuses when `INFO` reports `cluster_enabled:1`. That is a plain `redis://`
  URL to a cluster node.

This task makes a Cluster *connectable* without yet letting a user in. Task 5 opens the gate.

Two more places need the cluster's shape:

- The title bar (`render/mod.rs` `title_bar`, DESIGN §2: target and Source are never
  sacrificed) has to say what the target is.
- `core/state/copy.rs` builds the `redis-cli` command that `C` copies. Today it emits
  `redis-cli -h … -p … -n …`. On a Cluster it needs `-c` to follow `MOVED`, and it must drop `-n`,
  because a Cluster has only db 0.

## Decisions

1. **The gate is a `ClusterSupport` value threaded into `connect_with`**, not a cargo feature and
   not an environment variable. Two values:
   - `Refuse` is what `main.rs` passes until task 5;
   - `Allow` is what the integration tests pass.

   A cargo feature would split the binary's behaviour from the tested one, and an environment
   variable would be a hidden user-reachable switch. Task 5 flips `main.rs` and deletes `Refuse`
   if nothing else needs it.
2. **A plain `redis://` URL to a cluster node** (the `cluster_enabled:1` path) under `Allow`:
   **reconnect as a cluster client** using the same host and port as the seed. If that redial
   fails, keep the ADR-0021 diagnostic. The alternative was to stay refused and tell the user to
   use `redis-cluster://`. **Decided (user, 2026-10-05): redial**, because a user who pastes a
   node address expects the cluster.
3. **The Cluster's shape is core state**: a `Topology { primaries: u16, nodes: u16 }` on
   `Connection`, `None` for a non-cluster target. The shell fills it from
   `cached_cluster_state()` after connecting, and refreshes it on `cluster_change_rx()` with a
   `Msg::TopologyChanged`. The core never asks for it.
4. **Title bar:** `cluster · 3 primaries · 6 nodes` sits after the target and yields before it
   when space is short. The Environment and Source pair is still never sacrificed (DESIGN §2).
   The db segment is not shown for a Cluster.
5. **`C` on a Cluster:** `redis-cli -c -h <seed host> -p <seed port> <verb> <key>`, with no `-n`.
   It uses the seed, not the owner, because `-c` follows the redirect and the seed is what the
   title bar shows.
6. **`infer_environment` is unchanged.** M4 task 1 already strips `-cluster` schemes.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/mod.rs` | `ClusterSupport`; `build_config` and `server_conditions` honour it; the redial on `cluster_enabled:1` |
| `crates/app/src/main.rs` | passes `Refuse` |
| `crates/core/src/state/…` (`Connection`) | `topology: Option<Topology>` |
| `crates/core/src/msg.rs` | `Msg::TopologyChanged` |
| `crates/core/src/render/mod.rs` | title-bar segment and golden frames (wide, narrow, ASCII) |
| `crates/core/src/state/copy.rs` | `-c`, no `-n` on a Cluster; unit tests |
| `crates/app/src/terminal.rs` | listener on `cluster_change_rx()` that sends `TopologyChanged` |

## Testing

- **Golden frames:** the title bar with a topology at 140, 80 and 60 columns, showing the segment
  yielding before Environment and Source.
- **Unit tests:** the `C` payload on a Cluster.
- **Integration tests** (task 1 harness):
  - `Allow` connects through `redis-cluster://` and through a plain URL to one node, and both
    report 3 primaries and 6 nodes;
  - `Refuse` still produces the ADR-0021 diagnostic on both paths, with the existing tests
    unchanged.

## CLAUDE.md rules this binds

- The title bar shows target and Source at all times.
- The core owns no I/O: the topology arrives as a `Msg`.
- ADR-0005: a Cluster is one logical target, with nothing to switch.

## Out of scope

Scanning (task 3), arming (task 4), and lifting the gate for users (task 5).

## Outcome

Done. A Cluster target connects behind a gate that is still closed to users; `main.rs` passes
`ClusterSupport::Refuse`, so the shipped behaviour (the ADR-0021 diagnostic) is unchanged. Implements
R1.11 and ADR-0022 (still Proposed).

- **Gate and redial:** `ClusterSupport { Refuse, Allow }` is threaded into `build_config`,
  `connect_with`, `open_feed` (and so the Monitor and Subscribe feeds) and the shell's reconnect.
  `connect()` stays the credential-less `Refuse` convenience. Under `Allow`, `redis-cluster://`
  connects as a cluster client; a plain URL whose `INFO` says `cluster_enabled:1` is redialled to the
  same seed through `build_config`, converting its `Centralized` server to `Clustered` so credentials
  and TLS carry over. A failed redial returns the ADR-0021 `ConnectError::Cluster`. Monitor's own
  clustered refusal is untouched.
- **Topology:** `Topology { primaries, nodes }` on `Connection` (`Option`), `Msg::TopologyChanged`,
  `Established::topology`. The shell reads it with `redis::topology_of` (from
  `cached_cluster_state()`) after connecting and on a reconnect; a listener on `cluster_change_rx()`
  inside `Shell::watch_link` re-sends it, so it is re-subscribed wherever `watch_link` runs.
- **Render and copy:** the segment `cluster · 3 primaries · 6 nodes` follows the target, is drawn only
  when the whole target and segment fit, and goes through the glyph set. A Cluster shows no `/db`.
  `redis_cli_command` takes a `cluster` flag: `-c -h <seed> -p <port>` (plus `--tls` for a `rediss`
  scheme), no `-n`. DESIGN §2 updated. Goldens: `title_bar_cluster` (140/120/100/80/60 and ASCII).
- **Integration tests** (in `cluster_harness`): `Allow` by both URLs reports 3 primaries / 6 nodes;
  `Refuse` through `connect_with` keeps the diagnostic on both; a failover reaches `cluster_change_rx`.

### Deviations

- **Cargo features:** `fred` now enables `i-cluster` (for `cached_cluster_state`) and `replicas` (so
  `SlotRange::replicas` exists, needed to count nodes).
- **`read_only` on a cluster client is never `Replica`.** A keyless `INFO` goes to whichever node `fred`
  picks, so one replica answering would mark a healthy Cluster as a replica. Task 6 owns the real rule.
- **`open_feed` takes a `ClusterSupport`** (and carries `#[allow(clippy::too_many_arguments)]`).
- The `terminal.rs` listener has no automated test (it needs a terminal); the same `cluster_change_rx`
  and `topology_of` are exercised directly by the failover test.

### Findings about `fred` 10.1.0 for tasks 3-4

- `cached_cluster_state()` is a list of `SlotRange { start, end, primary, id, replicas }`, from `CLUSTER
  SLOTS`. **Redis omits a replica whose replication offset is still 0**, so a freshly formed, idle
  cluster advertises 3 nodes, not 6, until it has seen a write (the tests `warm` it). The node count is
  therefore "what CLUSTER SLOTS lists", and nothing fires `cluster_change_rx` when a replica later
  appears.
- `cluster_change_rx` **does fire on a forced failover**, once the client notices (it needs traffic or a
  sync): `[Add(new primary), Remove(old primary)]`. After it, `cached_cluster_state()` converges to 3
  primaries / 6 nodes.
- The cluster client's keyless commands (`INFO`) are not pinned to a node.
