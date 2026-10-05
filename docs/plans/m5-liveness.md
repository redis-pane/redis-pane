# M5 task 4: Owner-pinned liveness

Status: **done.** This is M5's riskiest task: liveness is the headline feature, and a wrong
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

## Outcome

Done. A Cluster's open key is armed on its owner, and only the owner's reconnect, or the slot
changing owner, drops `● live`. Implements R1.11 and ADR-0022 (still Proposed); the refusal gate in
`main.rs` is unchanged, so no user sees this until task 5.

- **Spike (phase 1): passed.** Pinned arming lands on the owner for a key on each primary, 5
  quiet and 20 noisy re-arms each, under 4 concurrent traffic tasks; every `Invalidation::server`
  was the owner; `CLIENT TRACKINGINFO` shows `caching-yes` and no non-owner held one. The unpinned
  control lost 10 of 15 armings, so the spike discriminates. Evidence and findings are in
  ADR-0022's arming section.
- **`read.rs`:** `arming_pipeline` (`Pipeline::from(client.with_options(..))`, pinned only on a
  Cluster client, so one read path) and `arm_and_type`, which re-sends the arming if the slot's
  owner changed across the pipeline (stale table).
- **`crates/app/src/liveness.rs` (new):** `OpenOwner` (the shell's record of the owner),
  `watch_reconnects` (owner or non-Cluster: re-probe, `ServerState`, `Connected`; any other node:
  re-probe tracking only), `watch_topology` (`cluster_change_rx` plus a 1s `sync_cluster` while a
  key is armed; owner changed: `Connected`), `watch_errors` (the former inline `error_rx` task,
  moved unchanged) and `is_wedged`. `terminal.rs` calls them from
  `watch_link`, records the owner in `read_key` before sending `TrackingArmed`, and clears it in
  `reconnected`.
- **Core:** no change. No new message and no node type; `Msg::Connected` is the message a reconnect
  and an owner change both send.
- **Tests** (in `cluster_liveness`, all `#[ignore]`d, on the task 1 harness, 9 in all): the spike and
  its negative control; armed on the owner for a key on each primary in a loop of 5; no non-owner
  holds a pending opt-in; an unrelated node's reconnect, on a test-only client that has a reconnect
  policy (no refetch, still live, no redial, tracking re-enabled on it, a later write still
  invalidates); an unrelated node's connection killed on the app's own client (redial, then live and
  armed again); the owner killed (live drops, re-arms, later write invalidates); forced failover
  (0 lost links, re-armed through the topology path); slot migration. The tests drive the real `liveness`
  listeners, `read_value` and `OpenOwner`, with a small stand-in for the core's two handlers (and
  a redial on `ConnectionLost`), because `terminal.rs` needs a terminal. The core's own liveness
  unit tests are unchanged and pass.

### Deviations

- **"Unrelated node reconnects, no refetch" is not reachable on the app's client.** The plan
  assumed `fred` reconnects one node in place. It does not without a reconnect policy, and the client
  is dead until redialled (ADR-0022). A Cluster-wide policy fixed that but made `fred`'s router spin
  with a node down, hanging task 3's dead-primary scan test, so it was reverted. Any node's drop is
  therefore a lost link and a redial, which refetches and re-arms (safe, never a false `● live`).
  The non-owner filter in `watch_reconnects` is built and tested against a policy client, for
  when one exists. `Shell::reconnected` now `quit()`s the replaced client.
- **A wedged client is reported as a lost link** (`Routing` or `Timeout` on a read, Cluster only),
  the only recovery from the `fred` ASK wedge in ADR-0022. The migration test passes because of it:
  the key's own `MIGRATE` invalidates it, the refetch hits `ASK`, the client wedges, the redial
  recovers.
- **`arm_and_type`'s owner comparison** (not in the plan) for the stale-table mis-arm.
- The spike tests stay in the suite as regression tests.

### For tasks 5-9

- Any read of a key whose slot is being migrated wedges the main client permanently (`fred`
  10.1.0). Anything else that reads through the main client during a rebalance inherits that;
  `liveness::is_wedged` is the detector. Task 6 (mutations on a Cluster) in particular.
- `reconnect_rx` never fires on the app's Cluster client (no policy). Do not add a policy without
  first re-running task 3's `a_dead_primary_fails_the_scan_naming_it`, which it hangs.
- `OpenOwner::armed` is the one place the owner is recorded; a new surface that arms must call it.
