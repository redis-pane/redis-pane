# M5 task 1: Cluster test harness + ADR-0022

Status: **planned.**

## Context

[`m5-planning.md`](m5-planning.md) row 1. Every later M5 task proves itself against a real
cluster, and none exists in the suite today. `crates/app/tests/integration.rs`'s
`start_cluster_enabled()` starts a single `--cluster-enabled yes` node. That is enough for the
ADR-0021 refusal test and nothing else. [`m5-cluster.md`](m5-cluster.md) § Testing names the
pitfall: cluster nodes announce addresses that the client then dials (`MOVED`, `CLUSTER SLOTS`),
so those addresses must be reachable from the test process. That is fiddly under Docker,
especially on Docker Desktop for macOS, where container IPs are not routable from the host.

## Decisions

1. **One container, six `redis-server` processes, identity port mapping.** At test time the
   harness picks six free host ports (bind `127.0.0.1:0`, read the port, release it). It starts
   one `redis:8.4-alpine` container that runs six nodes listening on exactly those ports, each
   mapped `host:N → container:N`, with `cluster-announce-ip 127.0.0.1`.
   - Inside the container the nodes reach each other on 127.0.0.1 (the bus ports, `N+10000`,
     are never mapped).
   - Outside it the test process reaches every announced address as-is.
   - This works the same on Linux CI and on Docker Desktop.

   Rejected:
   - One container per node: announce addresses must then be reachable both between containers
     and from the host, which needs host networking, and that does not work on macOS.
   - A third-party cluster image: unpinned and unaudited, and it hides the configuration the
     tests depend on.

   **Confirm at build time** that `testcontainers` 0.28 allows a fixed host port
   (`with_mapped_port`). If it does not, use the same layout with ports chosen in a fixed range
   and a retry on collision.
2. **Shape: 3 primaries, 3 replicas** (`redis-cli --cluster create … --cluster-replicas 1
   --cluster-yes`, run inside the container). Replicas are needed for task 4's forced failover
   (`CLUSTER FAILOVER` on a replica) and for task 6's all-replicas reason. Ready means
   `CLUSTER INFO` reports `cluster_state:ok` on every node, polled with a deadline, never a
   sleep.
3. **Helpers live in a shared test module**, so tasks 2–9 do not each reinvent them.
   Candidates: `crates/app/tests/support/cluster.rs`, or a section of `integration.rs` if the
   suite is still one file; decide by what `integration.rs` already does for shared helpers.
   - `start_cluster() -> Cluster`, which holds the container and exposes the seed URL
     (`redis-cluster://127.0.0.1:N`) and the node list.
   - `key_in_slot_of(primary)`, which yields a key name hashing to a given primary.
     `fred::util::redis_keyslot` gives the slot.
   - `failover(replica)` and `migrate_slot(slot, from, to)`. Tasks 4 and 6 need these.
4. **ADR-0022 "Cluster is supported, in stages" is written now, as Proposed.** It supersedes
   ADR-0021 and amends ADR-0008. It records:
   - the staged opening (tasks 2–4 behind the refusal, task 5 lifts it for the keyspace, tasks
     7–9 open the server views);
   - the owner-pinned arming design (task 4);
   - the identity-port harness.

   It becomes Accepted in task 5, the moment users can reach a Cluster. ADR-0021's status line
   gets "Superseded by ADR-0022 (from M5 task 5)" in that same change, not now.
5. **No product code changes.** The refusal stays exactly as it is.

## Files touched

| File | Change |
|---|---|
| `crates/app/tests/integration.rs` (+ a support module if split) | `start_cluster`, slot helpers, failover/migrate helpers, and a smoke test |
| `docs/adr/0022-cluster-supported-in-stages.md` | new, Proposed |
| `.github/workflows/ci.yml` | only if the cluster needs more time or ports than the job allows |

## Testing

- **Smoke test (ignored, Docker):** `start_cluster()` reaches `cluster_state:ok`. A plain `fred`
  cluster client built from the seed URL can `SET`/`GET` one key per primary, which proves
  `MOVED` redirects resolve to reachable addresses. `CLUSTER NODES` lists 3 masters and 3
  slaves.
- **The existing refusal test now also runs against the real cluster:** connecting via
  `redis-cluster://` still fails with the ADR-0021 diagnostic.
- Run it locally (Docker Desktop) **and** in CI before calling the task done; the address problem
  only shows on one of them.

## CLAUDE.md rules this binds

- The integration suite uses `testcontainers` and owns its own containers. `scripts/` is not
  used here.
- Never `sleep` waiting for a server: poll a readiness condition with a deadline.

## Out of scope

Any product behaviour, which starts in task 2. Valkey cluster: same protocol, not separately
tested in M5.
