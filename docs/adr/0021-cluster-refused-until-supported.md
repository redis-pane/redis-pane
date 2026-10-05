# ADR-0021 — Cluster is refused at startup until it is supported

**Status:** Superseded by ADR-0022 (from M5 task 5) · **Date:** 2026-10-01

## Context

[ADR-0008](0008-sentinel-in-v1-cluster-deferred.md) deferred Cluster from v1 and named what real
support would have to solve: per-node `SCAN`, per-node `INFO`/`DBSIZE`/`SLOWLOG`, and per-node
`CLIENT TRACKING`. It did not say what happens today when a Cluster target is pointed at the app
anyway, because nothing enforced the deferral. M4 planning looked, and found that nothing does:

- **A `redis-cluster://` URL connects.** `crates/app/src/redis/mod.rs`'s `build_config` and
  `connect_with` have no Cluster refusal; `server_conditions` reads `INFO` and moves on. The app
  scans exactly one arbitrary primary with the keyspace source's single cursor and then reports
  the scan **complete**, because nothing told it there were other nodes to cover.
- **`CLIENT CACHING YES` is keyless and routes to an arbitrary node, while `TYPE` (and every
  other per-key read) routes to the key's owner.** So the Viewer can arm tracking against one
  node while reading the value from another, and still show `● live` — the exact ambiguity
  ADR-0006 was written to make impossible, reopened by a topology ADR-0006 never considered.
- **Dashboard and Slowlog each show one node's view, silently.** `INFO` and `SLOWLOG GET` answer
  for whichever node the main connection happens to be talking to; nothing on either screen says
  so. Pub/Sub is the partial exception: a classic `PUBLISH` is broadcast to every node in a
  cluster, so an ordinary `SUBSCRIBE` sees cluster-wide traffic — but sharded Pub/Sub
  (`SPUBLISH`) is not, and `feed.rs` drops `SMessage` as unreachable.
- **Only Monitor refuses.** `crates/app/src/redis/feed.rs`'s `monitor_config` checks
  `config.server.is_clustered()` and returns an error naming ADR-0008 before `fred::monitor::run`
  is ever called — proven by its own test,
  `a_clustered_config_is_refused_before_ever_reaching_fred`. Every other code path has no
  equivalent check.
- **A plain `redis://` URL to a cluster-mode node is the same bug wearing a different URL
  scheme.** Nothing about the misbehaviour above depends on the `redis-cluster://` scheme — it
  depends on the server being in cluster mode, which `INFO`'s `cluster_enabled:1` reports
  regardless of what scheme connected to it. A scheme-only check would miss this entirely.

So "Cluster is deferred" has meant, in practice, "Cluster is unsupported and undetected" — a
Cluster target does not fail, it silently answers a different, smaller question than the one
asked, exactly the shape of defect this project exists to not have (CLAUDE.md: "never swallow a
Redis error silently... the operation vanishes and nothing changes on screen, which is
indistinguishable from the app deciding to do nothing"). The gap is live today, not hypothetical,
which is why it is the highest-risk task in M4's task table.

## Decision

**Cluster stays out of M4 and becomes its own milestone, M5, after the beta** (decided
2026-10-03; PRD §9). Its design is [`plans/m5-cluster.md`](../plans/m5-cluster.md). The refusal
below is that plan's stage 0, and M5 will supersede it with an ADR of its own.

**Until M5 delivers Cluster support, a Cluster target is refused at startup with a clear diagnostic,**
rather than connected and silently served from one arbitrary node. Detection is two-layered, so
neither a URL nor a topology check alone has to carry it:

- **By URL scheme** — a `redis-cluster://` (or equivalent) target is refused before a connection
  is attempted at all.
- **By server response** — `INFO cluster`'s `cluster_enabled:1` on a target reached by a plain
  `redis://`/`rediss://` URL is refused once that fact is known, before the app proceeds to scan
  or render anything.

Either path produces the same shape of failure as every other startup refusal ADR-0009 already
specifies: a diagnostic naming the target, its Source, the cause, and this ADR, with a non-zero
exit — never a half-working session that looks connected while quietly covering one node.

## Alternatives considered

- **Leave it as is.** Rejected: this is the status quo, and the status quo is a Viewer that can
  claim `● live` over a value armed and read from different nodes, a scan that reports "complete"
  having seen one primary's keys, and three screens (Dashboard, Slowlog, Pub/Sub) silently
  describing one node as though it were the whole deployment. Silent wrong answers are worse than
  a refusal — this project exists because RedisInsight gets exactly this kind of thing wrong.
- **Warn, but allow it.** A banner reading "Cluster detected, some views may be incomplete"
  was considered and rejected: a banner does not make `● live` true, does not make the scan's
  "complete" honest, and does not tell the reader which of the three silently-single-node screens
  they happen to be looking at. It converts an unambiguous failure into an ambiguous one, which is
  the opposite of what every liveness and error-surfacing rule in this codebase (ADR-0006, R7.4)
  is for.
- **Build real Cluster support now.** The user considered and rejected this for M4, 2026-10-01:
  it is a large, separate piece of work — N-cursor merged scan, per-node dashboards, per-node
  tracking pinned to the key's owner — and M4's other scope (performance, themes, session
  restore) is independently valuable and does not need to wait on it. See "what real support must
  solve" below for the shape that future work takes.

## Consequences

- R1.11 and PRD §3's non-goal both now say Cluster is M5, not merely out of v1.
- `infer_environment` (`crates/core/src/resolve.rs`) strips only the `redis://` and `rediss://`
  prefixes today, so a loopback `redis-cluster://` or `redis-sentinel://` URL falls through to
  `Environment::Unknown` rather than `Local` — correct by accident for Cluster (which this ADR
  now refuses anyway, so its Environment is moot) but still wrong for Sentinel, a v1 target. The
  per-task plan for this ADR's implementation (`docs/plans/m4-cluster-refusal.md`) fixes
  `infer_environment` to recognise both schemes, so a loopback Sentinel URL resolves to `local` as
  every other loopback target does.
- **What real support must solve, when it is next picked up** — carried over from
  [ADR-0008](0008-sentinel-in-v1-cluster-deferred.md) and extended by this ADR's findings:
  - ADR-0008's three original items: the keyspace source's single cursor becomes N cursors merged
    behind the same "stream of keys with progress" abstraction (already load-bearing per ADR-0008,
    unchanged by this ADR); `INFO`/`DBSIZE`/`SLOWLOG` become per-node, which means the Dashboard
    and Slowlog need a node selector or an aggregation story they do not have today; `CLIENT
    TRACKING` must arm per-node.
  - **New, found here:** `CLIENT CACHING YES` must be pinned to the key's owner node, not issued
    on whatever connection happens to be open — the ambiguity this ADR's Context section
    describes is specifically a mismatch between where tracking arms and where the value is read.
  - **New, found here:** reconnect events become per-node, not the single reconnect
    ADR-0009 already handles — a Cluster client can lose and regain one node while the rest of the
    topology stays up, which the current reconnect/backoff/re-arm machinery has no concept of.
  - **New, found here:** `fred`'s `scan_cluster` stream may continue producing pages after one
    page reports `has_more() == false` — a known footgun worth designing the merged-cursor
    abstraction around explicitly, not discovering mid-implementation the way `set_value`'s fred
    footgun was discovered during M2 (see `crates/app/src/redis/mod.rs`'s doc comment on
    `set_value`, same category of surprise).

## Amended by

Nothing yet amends this ADR.

## Sources

- `crates/app/src/redis/mod.rs` — `build_config`, `connect_with`, `server_conditions` (no Cluster
  refusal today).
- `crates/app/src/redis/feed.rs` — `monitor_config`, the one existing refusal, and its test
  `a_clustered_config_is_refused_before_ever_reaching_fred`.
- `crates/core/src/resolve.rs` — `infer_environment`.
- [ADR-0008](0008-sentinel-in-v1-cluster-deferred.md), which this ADR amends (see its own
  "Amended by" line).
- [ADR-0009](0009-connection-lifecycle.md) — startup diagnostic shape this ADR's refusal reuses.
- [ADR-0006](0006-liveness-without-a-refresh-button.md) — the liveness ambiguity this ADR's
  `CLIENT CACHING` finding reopens at a different layer.
