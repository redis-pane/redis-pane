# M5 task 6: Mutations on a Cluster

Status: **planned.**

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
