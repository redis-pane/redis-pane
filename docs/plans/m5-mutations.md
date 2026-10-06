# M5 task 6: Mutations on a Cluster

Status: **done.**

## Context

[`m5-planning.md`](m5-planning.md), row 6. Scope decision (2026-10-05): **guard only what
exists.** Every shipped mutation is single-key, and every guarded script in
`crates/app/src/redis/mutate.rs` touches only `KEYS[1]`. So `fred` routes each one to the owner,
and none can raise `CROSSSLOT`. That is the claim; this task turns it into tests. It also fixes
the one place where read-only state is decided per server rather than per cluster:
`server_conditions` derives `ReadOnlyReason::Replica` from `INFO`'s `role`, which on a cluster
client is whatever node answered.

## Decisions

1. **Integration-test every shipped mutation on the cluster**, with one key in each primary's
   slot range (task 1's `key_in_slot_of`). The mutations: string edit, hash field edit, add and
   remove, set member, list element, zset score, TTL set and clear, and key delete. Every guarded
   script is covered, including the "key gone" and "field taken" refusals, so the script's
   `KEYS` declaration is proven correct on a cluster.
2. **Audit `mutate.rs` for any key used in a script without being passed in `KEYS`.** On a
   cluster that is a silent wrong-node write. If one exists, fix it; it is a latent bug anyway.
3. **The `replica` reason on a Cluster.** It applies only when every reachable node is a
   replica. A cluster client's writes go to primaries, so a normal cluster is never locked as
   `replica`. The shell computes this from `cached_cluster_state()`. The core keeps its existing
   `ReadOnlyReason` and is not told why. The `environment` and `user` reasons are unchanged.
4. **Write the Cluster rules into PLAN §5's rows for M2 tasks 11–13**, so they are built with
   those features:
   - **Rename:** the preview warns when source and target hash to different slots, because
     `RENAME` across slots fails with `CROSSSLOT`.
   - **Copy:** the same warning. A cross-slot copy falls back to `DUMP` then `RESTORE … REPLACE?`
     behind the same confirm dialog, with TTL carried over.
   - **Bulk delete:** keys are grouped by node. The typed confirmation on `prod` counts every key
     across every node.
   - **Move across databases:** hidden on a Cluster, which has only db 0.

## Files touched

| File | Change |
|---|---|
| `crates/app/tests/integration.rs` | per-primary mutation tests |
| `crates/app/src/redis/mod.rs` | the `replica` reason on a Cluster |
| `crates/app/src/redis/mutate.rs` | only if the audit finds an undeclared key |
| `docs/PLAN.md` §5 | Cluster notes on rows 11–13 |

## Testing

- The integration tests above.
- **The `replica` reason:**
  - a cluster where only the seed node is a replica is **not** locked;
  - a seed URL pointing at a replica is **not** locked;
  - an all-replica topology is locked as `replica`. This may not be reachable with a real
    cluster; if not, unit-test the decision function.

## Out of scope

Rename, copy, bulk and hash-field rename themselves (M2 tasks 11–14).

## Outcome

Done. Every shipped mutation is proven on a real Cluster, and the `replica` reason is decided per
Cluster. Implements R1.11, R4.x (the mutation chokepoint) and ADR-0022.

- **Audit of `mutate.rs`: clean, no change needed.** Every guarded script (hash field edit and add,
  set member add, list element edit, add and delete, zset score edit and add, TTL persist and shift)
  reads and writes only `KEYS[1]`; no key name is built in Lua or passed in `ARGV`. The one
  non-data `ARGV` entry is the literal `LPUSH`/`RPUSH` command name in the list-add script. The
  plain commands (`SET`, `DEL`, `HDEL`, `SREM`, `ZREM`, `EXPIRE`) are single-key. The one keyless
  mutation, `SLOWLOG RESET`, is unreachable on a Cluster (task 5 makes the Slowlog view inert).
- **Tests** (`cluster_mutations` in `integration.rs`, `#[ignore]`d): every mutation succeeds on a
  key in each of the three primaries' ranges; every refusal path (key gone for all 13 guarded
  writes, field taken or gone, member taken or gone, element moved, no expiry, would expire now,
  nothing to remove) holds on each primary and never recreates a key; a write to a slot
  mid-migration fails with the server's detail and `link_lost`, and a fresh client works once
  ownership settles; a Cluster seeded through a replica (by `redis-cluster://` and by `redis://`)
  is not locked and can write. The existing single-node replica test still locks as `replica`.
  Five unit tests cover `cluster_is_all_replica` (all-replica, seed-replica with a
  primary, each unreachable-primary flag, empty or garbage reply, a normal cluster).
- **Wedge on the write path.** `mutate::execute_settled` wraps `execute` and sets `link_lost` with
  `liveness::is_wedged`; `Shell::mutate` sends `Msg::ConnectionLost` after `MutationSettled`, as the
  read path does. No reconnect policy was added.
- **Harness.** `Cluster::failover` retries `CLUSTER FAILOVER` (as `FORCE` from the second attempt)
  every 20s under a 120s deadline, after the 60s deadline timed out twice under host load.
  `migrate_slot` is split into `begin_migration` and `finish_migration` so a test can stand in a
  mid-migration slot.
- **Stream test.** `a_freshly_added_entry_reads_as_just_added` compared the stream ID's timestamp
  (the container's clock) with the host's; Docker Desktop's VM clock drifts from the host's. It now
  takes "now" from the server's `TIME`.

### Deviations

- **The replica rule reads `CLUSTER NODES`, not `cached_cluster_state()`.** The routing table lists
  only primaries, so it can never report "no primary". A failing `CLUSTER NODES` fails the
  conditions probe as a failing `INFO` does (surfaced, never defaulted).
- All-replica is covered by the pure function only: a real cluster with no primary is not reachable
  through the harness.
- PLAN §5 rows 11 to 13 carry the Cluster rules (rename, copy, bulk delete, move-across-db hidden).
