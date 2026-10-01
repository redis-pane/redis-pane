# M4 task 1: Refuse Cluster targets

Status: **planning — not started.**

## Context

PLAN.md M4 row 1: "Refuse Cluster targets · Proves: a `redis-cluster://` URL or a Cluster-mode
server (detected via `INFO cluster`'s `cluster_enabled:1`, so a plain `redis://` URL to a cluster
node is caught too) fails at startup with a diagnostic naming ADR-0021, never a half-working
session. `infer_environment` recognises `-sentinel`/`-cluster` schemes, so a loopback Sentinel URL
is `local`, not `unknown`. Integration test against a real `cluster-enabled yes` container."

[ADR-0021](../adr/0021-cluster-refused-until-supported.md) is the decision this task implements.
Its Context section is the full account of what is broken today; summarized here as the concrete
code facts that drive the fix:

- `crates/app/src/redis/mod.rs`'s `build_config` and `connect_with` have no Cluster check at all.
  A Cluster target connects, `server_conditions` (which reads `INFO`) runs, and nothing downstream
  refuses.
- `crates/app/src/redis/feed.rs`'s `monitor_config` is the **only** existing refusal:
  `if config.server.is_clustered() { return Err(...) }` before `fred::monitor::run` is called, with
  its own test `a_clustered_config_is_refused_before_ever_reaching_fred`. This task generalizes
  that check to apply before a session is allowed to start at all, not just before Monitor opens.
- `crates/core/src/resolve.rs`'s `infer_environment` only strips `redis://` and `rediss://` before
  checking for loopback/unix-socket, so a loopback `redis-sentinel://` or `redis-cluster://` URL
  does not match `is_loopback`/`is_unix_socket` and falls through to `Environment::Unknown`. For
  Sentinel (a real v1 target) this is a real bug independent of this task; for Cluster it has been
  masked by the fact that nothing refuses a Cluster target today, so its Environment has never
  mattered.

## Decisions

1. **Detection is two layers, both refusing before the session starts rendering anything:**
   - **URL scheme.** A `redis-cluster://` (or `rediss-cluster://`, if `fred`/the URL parser
     recognizes it — **confirm at build time** which scheme strings `fred::Config::from_url`
     actually accepts for a Cluster target) is refused before any connection attempt, the cheapest
     and earliest check.
   - **Server response.** A target reached by a plain `redis://`/`rediss://` URL that turns out to
     report `cluster_enabled:1` in `INFO cluster` (or `INFO server`'s `redis_mode` field — **confirm
     at build time** which `INFO` field is more reliable; Redis exposes cluster state both in
     `INFO server`'s `redis_mode:cluster` and `INFO cluster`'s `cluster_enabled:1`) is refused once
     that fact is known — i.e., folded into the same `server_conditions` call `connect_with`
     already runs, so no second round trip is added.
2. **The refusal is a startup diagnostic, not a runtime error surfaced mid-session.** It reuses the
   exact shape ADR-0009 already specifies for "target unreachable at launch": target, Source, cause,
   non-zero exit. The cause names ADR-0021 explicitly (e.g. `Cluster targets are not supported —
   see ADR-0021`), the same way other startup diagnostics name the deciding ADR where one exists.
3. **`infer_environment` gains `-sentinel`/`-cluster` scheme recognition**, stripping
   `redis-sentinel://`, `rediss-sentinel://`, `redis-cluster://`, `rediss-cluster://` the same way
   it already strips the two plain schemes, before the loopback/unix-socket check. This fixes the
   Sentinel bug PLAN's "Proves" column calls out; for Cluster it is moot in practice once this task
   ships (a Cluster target never reaches the point where its Environment is used), but the function
   should still answer correctly for it rather than relying on the refusal happening first by
   accident.
4. **`monitor_config`'s existing Cluster check stays** — it is cheap, it is already tested, and a
   second independent check at the feed layer is a reasonable defense in depth even though the
   top-level startup refusal should mean it never fires in practice.
5. **Wording.** The diagnostic should say what a reader can do about it, not just that it failed —
   propose something like: `redis-pane does not support Cluster yet (ADR-0021). Target: <url>
   (Source: <source>). Point at a single node or a Sentinel-fronted deployment instead.`
   **Confirm the exact wording at build time** against ADR-0009's established diagnostic format.

## Architecture

All of this is shell-side. `crates/core` has no network access and cannot itself probe
`cluster_enabled`, so the core's only role is in `infer_environment` (already core, pure, testable
without a server) and in whatever startup-diagnostic formatting function already exists for other
launch-time refusals (reuse it; do not add a second one).

- `crates/app/src/redis/mod.rs`: `build_config` gains the URL-scheme check (fails fast, no
  connection attempt). `connect_with` (or `server_conditions`, whichever already owns the
  post-connect `INFO` read) gains the `cluster_enabled:1` check, surfaced as the same
  `ConnectError` variant family the version-floor refusal (ADR-0007) already uses, so `main.rs`'s
  existing startup-diagnostic formatting picks it up without a new arm.
- `crates/core/src/resolve.rs`: `infer_environment`'s scheme-stripping list grows by two entries.
- `crates/app/src/redis/feed.rs`: unchanged — `monitor_config`'s check stays as a second line of
  defense.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/mod.rs` | URL-scheme refusal in `build_config`; `cluster_enabled:1` refusal alongside `server_conditions`/`connect_with` |
| `crates/core/src/resolve.rs` | `infer_environment` strips `-sentinel`/`-cluster` scheme variants too |
| `crates/app/src/main.rs` | diagnostic wording for the new `ConnectError` variant, if not already generic enough to need no change |
| `docs/adr/0021-cluster-refused-until-supported.md` | already written; update only if build-time findings change the detection mechanics described there |

## Testing

- **Core unit tests** (`crates/core/src/resolve.rs`): `infer_environment("redis-sentinel://127.0.0.1:26379")`
  and the `redis-cluster://` equivalent both resolve to `Environment::Local` for a loopback host,
  matching every other loopback scheme; non-loopback cases stay `Unknown`.
- **App-level unit test**: the URL-scheme refusal fires in `build_config` without attempting a
  connection (no Docker needed — this is a pure string/parse check).
- **Docker-backed integration test**: start a `cluster-enabled yes` single-node container (per the
  task brief's suggestion — `GenericImage::new("redis", <tag>).with_cmd(vec!["redis-server",
  "--cluster-enabled", "yes"])`, the same `with_cmd` mechanism `crates/app/tests/integration.rs`
  already uses for `--requirepass` and replica setups) and prove: a plain `redis://` URL to it is
  refused with a diagnostic naming ADR-0021, not a Cluster-scheme URL (a single-node cluster-enabled
  container has no multi-node topology, so this tests the `cluster_enabled:1` path specifically,
  which is the harder of the two to get right); the existing `monitor_config` Cluster test still
  passes unchanged.

## CLAUDE.md rules this binds

- **Errors surface as non-blocking notifications carrying the failing command** — this is a
  startup refusal rather than a mid-session notification, but the same spirit: never a silent
  degrade, never a swallowed `Err(_) => return`.
- **Connection resolution... decisions already made.** This task does not change the resolution
  chain's precedence, only adds a refusal downstream of it, consistent with "the title bar shows
  target and Source at all times" — a refused Cluster target still gets a diagnostic naming both.

## Out of scope

- **Any actual Cluster support** — N-cursor scan, per-node Dashboard/Slowlog, per-node tracking.
  ADR-0021's "what real support must solve" section is the record for when that work is picked up;
  this task is the refusal only.
- **Sentinel behaviour changes beyond the `infer_environment` fix** — Sentinel already ships in v1
  (ADR-0008) and this task does not touch its connection or failover handling.
