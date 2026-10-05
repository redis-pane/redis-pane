# M5 task 4: Owner-pinned liveness

Status: **planned.** This is M5's riskiest task: liveness is the headline feature, and a wrong
arming here reopens the RedisInsight bug the project was started over (ADR-0006).

## Context

[`m5-planning.md`](m5-planning.md), row 4. Today:

- **The arming:** `crates/app/src/redis/read.rs` `read_value` sends `CLIENT CACHING YES` and
  `TYPE` as one pipeline, so nothing can slip between them on the connection.
  - On a cluster client, `CLIENT CACHING` has no key, so fred routes it to an arbitrary node.
    `TYPE` goes to the owner.
  - The arming therefore lands on the wrong connection, and `● live` is shown over a key nothing
    tracks.
- **Tracking:** `start_tracking(…, bcast=false, optin=true, …)` already sends `CLIENT TRACKING ON
  OPTIN` to every node (`_ClientTrackingCluster`). So every node connection is opted in, and only
  the arming is mis-routed.
- **Reconnects:** `terminal.rs` `watch_link` listens on `reconnect_rx()`, which yields the
  `Server` that reconnected, and treats any reconnect as "re-arm". On a cluster that refetches on
  every unrelated node's reconnect.
- **Invalidations:** `Invalidation { keys, server }` names the sending node.

## Decisions

1. **Pin the arming pipeline to the key's slot.** Build the pipeline from
   `client.with_options(&Options { cluster_hash: Some(ClusterHash::Custom(redis_keyslot(name))),
   .. })` so both `CLIENT CACHING YES` and `TYPE` carry the same slot and go to the owner's
   connection. On a non-cluster client the option is a no-op, so the read path stays one path.
   Do **not** use `Options.caching`: `read.rs` records it as inert in 10.1.0.
2. **Phase 1 is a spike that proves decision 1 against the real cluster before anything is built
   on it.** Two things to prove:
   - the pinned pipeline arrives on the owner's connection, with nothing interleaved;
   - a write from a second client to the key produces an `Invalidation` whose `server` is the
     owner.

   If fred splits or reorders a pinned pipeline, stop and come back with options; the plan
   changes then. Candidates are `with_cluster_node(owner)` plus a per-node lock, or a dedicated
   per-node connection for the Viewer.
3. **The Viewer records its owner.** The shell resolves the owner `Server` for the slot from
   `cached_cluster_state()` at arming time, keeps it beside the open key, and refreshes it on
   every re-arm. The core gets no node type: it keeps seeing `Msg::Invalidated` and reconnect
   messages exactly as today, and the shell decides which ones concern the open key.
4. **Only the owner's reconnect re-arms.** `watch_link` compares the reconnecting `Server` with
   the recorded owner:
   - a match goes through today's path (drop the liveness claim, re-probe, refetch and re-arm,
     ADR-0009);
   - any other node is ignored for liveness.
5. **A key that moves is re-armed on its new owner.** On `cluster_change_rx()` (failover or
   rebalance), the shell recomputes the slot's owner. If it changed, it sends the same message a
   reconnect does: the core drops `● live`, then refetches and re-arms. Until that refetch lands
   the header shows the degraded state, never `● live`. A `MOVED`/`ASK` on the refetch is
   followed by fred and needs nothing here.
6. **ADR-0006's guarantee is the acceptance test, not an aspiration.** `● live` is never shown
   over a key that no node is tracking for this client. It holds after an invalidation, after
   the owner reconnects, after an unrelated node reconnects, after a forced failover and after a
   slot migration.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/read.rs` | slot-pinned arming pipeline |
| `crates/app/src/terminal.rs` | owner tracking; reconnect filter; cluster-change → re-arm |
| `crates/core/…` | ideally nothing; at most a reason variant on the existing re-arm message |
| `docs/adr/0022-…` | arming section updated with what the spike proved |

## Testing

All of these are integration tests on the task 1 harness:

- **Armed on the owner.** For one key per primary, open it, change it from a second client, and
  the Viewer refetches. Run it in a loop of 5, because the arming is consumed each time.
- **Not armed elsewhere.** `CLIENT TRACKINGINFO` on each non-owner connection shows no pending
  opt-in for that key. **Confirm at build time** whether that is observable; if it is not, prove
  it through "invalidation server == owner".
- **Unrelated node reconnects.** `CLIENT KILL` this client on a non-owner node: no refetch, and
  liveness is still `● live`.
- **Owner reconnects.** `CLIENT KILL` on the owner: `● live` drops, the key is re-armed, and a
  later write still invalidates.
- **Forced failover.** `CLUSTER FAILOVER` on the owner's replica: the key re-arms on the new
  primary, and a later write invalidates.
- **Slot migration.** Moving the key's slot to another primary has the same outcome.
- **Unit tests:** the core still never claims liveness without an arming. These tests already
  exist; extend them if a message variant is added.

## CLAUDE.md rules this binds

- The Viewer never caches a value; there is one read path, and it re-arms every time.
- Tracking is consumed by its own invalidation (ADR-0006).
- Two re-arm invariants, after every invalidation and after every reconnect (ADR-0009). Here the
  reconnect is the owner's.
- Every connection path goes through `connect_with`.

## Out of scope

Server views (tasks 7–9). Opening the gate to users (task 5).
