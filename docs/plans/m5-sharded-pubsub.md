# M5 task 9: Sharded Pub/Sub

Status: **planned.**

## Context

[`m5-planning.md`](m5-planning.md) row 9. Pub/Sub (`g p`, [`m3-pubsub.md`](m3-pubsub.md)) uses a
`SubscriberClient` in `feed.rs`, with `a` subscribing to a channel or a pattern. Classic
`PUBLISH` is broadcast to every node of a Cluster, so ordinary subscriptions already see
cluster-wide traffic, and task 5 leaves Pub/Sub open for that reason.

Sharded Pub/Sub (`SPUBLISH`/`SSUBSCRIBE`, Redis 7+) is not broadcast. It is delivered only on the
node that owns the channel's slot. Today `feed.rs` drops any `MessageKind::SMessage` it receives.
`SubscriberClient::ssubscribe` exists in fred 10.1.

## Decisions

1. **Add a third subscription kind next to channel and pattern: sharded.**
   - The `a` input offers it with a toggle in the add form. That is a form field, not a new key.
   - The chip shows a "sharded" marker drawn from the glyph set, so it has an ASCII variant.
   - Sharded patterns do not exist in Redis, so the form does not offer the pattern kind together
     with sharded.
2. **Routing:** `ssubscribe` hashes the channel to its slot and subscribes on the owner. That is
   fred's behaviour; **confirm at build time** that it re-subscribes on the new owner after a
   failover. If it does not, the shell re-issues the subscription on `cluster_change_rx()`, the
   same pattern as task 4's re-arm.
3. **`SMessage` is shown**, in the same feed as classic messages, marked as sharded.
4. **Gating:**
   - Below Redis 7 the sharded toggle is disabled, and the reason ("needs Redis 7") is shown in
     help and in the form.
   - On a non-cluster Redis 7+ server, sharded subscriptions still work and behave like classic
     ones. Allowing them there keeps one code path.
5. **Nothing stays subscribed after leaving the view**, as today. `SUNSUBSCRIBE` is part of the
   teardown.

## Files touched

| File | Change |
|---|---|
| `crates/app/src/redis/feed.rs` | `ssubscribe`/`sunsubscribe`, `SMessage` delivery |
| `crates/core/src/state/pubsub.rs` (or equivalent), `render/…`, `theme` glyphs | sharded kind, chip marker, version gate |

## Testing

- **Integration tests, on the cluster:**
  - a sharded subscription receives an `SPUBLISH` sent to its channel;
  - a classic subscription does not receive it, and the reverse also holds;
  - after `CLUSTER FAILOVER` of the channel's owner, the subscription still delivers;
  - leaving the view leaves no subscriptions (`PUBSUB SHARDNUMSUB`).
- **Integration tests, on a single Redis 7+ node:** sharded subscriptions work.
- **Golden frames:** the sharded chip in Unicode and in ASCII, and the disabled toggle on Redis 6.
