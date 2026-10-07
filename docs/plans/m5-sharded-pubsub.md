# M5 task 9: Sharded Pub/Sub

Status: **done.**

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

## Outcome

Done. `g p` offers sharded subscriptions, on a Cluster and on any single Redis 7+ node. Implements
R6.2, R1.11 and ADR-0022.

- **Spike result (the build-time check decision 2 asked for), against a real 3-primary Cluster.**
  `SubscriberClient::ssubscribe` hashes the channel and subscribes on the **slot owner** only
  (`CLIENT LIST` `ssub=1` and `PUBSUB SHARDNUMSUB` = 1 on that one node, 0 on the other five), and an
  `SPUBLISH` from another client arrives as `MessageKind::SMessage`; a classic `PUBLISH` on the same
  name does not. After `CLUSTER FAILOVER` of the owner, **`fred` does not move the subscription**:
  the demoted node drops it and no node holds it (`SHARDNUMSUB` 0 on all six), nothing was delivered
  over 8s of `SPUBLISH`es, and `sync_cluster()` did not change that. An explicit `ssubscribe` after
  the sync landed on the new owner at once. So the shell re-issues it (below), as decision 2 allowed.
  Also learnt: `ssubscribe` with channels in different slots is `CROSSSLOT`, so the shell subscribes
  one channel per call.
- **Core.** `Subscription::Sharded(name)` joins `Channel` and `Pattern`; `Msg::PubSubMessage` and
  `PubSubMessage` carry `sharded`, set by the shell from the message kind (`push_message`;
  `push_pubsub_message` is the classic shorthand, one cap enforcement point). The `a` form gained a
  field: `Tab` toggles **sharded** (`PubSubState::sharded`, cleared whenever the form opens or
  closes). No new top-level key, so the keymap growth rule is untouched. Sharded excludes pattern:
  with the toggle on, the typed text is always one literal sharded channel (`*`, `?` and `[` are
  ordinary characters, a leading `=` is still stripped), via `parse_subscription_with`.
- **Gate.** `State::sharded_available()` reads the version probed at connect (`Link::Up.version`);
  below 7 `Tab` does nothing, the form shows `[-] sharded · needs Redis 7`, and help says
  `sharded: needs Redis 7`. An unknown version (before the first connect, or while reconnecting)
  is allowed, because the gate explains a known refusal and a server that refuses anyway reports it
  as a failed command. `connected()` now keeps the earlier version when a Cluster owner-change
  re-arm reports none, so that re-arm cannot erase a Redis 6 reading. The shell also cannot open a
  sharded feed on Redis 6: the server's refusal is a `ConnectError`.
- **Glyphs.** New role `Glyph::Sharded`: `§`, ASCII `#`, one column both ways. Chips read
  `[orders §]`; a sharded message's CHANNEL cell and detail head carry the same marker, so a sharded
  `orders` and a classic `orders` can be told apart in one feed.
- **Shell (`feed.rs`).** `open_subscribe` and `update_subscription` `SSUBSCRIBE` one channel per call
  and `SUNSUBSCRIBE` on removal; `SMessage` is delivered with `sharded: true` instead of dropped.
  `FeedHandle` keeps the held sharded channels with the owner each landed on. On a Cluster a watcher
  task wakes on `cluster_change_rx()` and every second (the same cadence as `liveness::watch_topology`),
  on the timer calls `sync_cluster()` (`fred` otherwise notices a failover late), and re-issues any
  `SSUBSCRIBE` whose slot owner changed; the owner is recorded only after the re-issue succeeded, and a
  failure is a `Msg::Failed` notification naming `SSUBSCRIBE <channel>`, retried on the next wake. No
  reconnect policy was added (ADR-0022). `close()` sends `SUNSUBSCRIBE` for each held sharded
  channel (2s bound each) before `QUIT`.
- **Tests.** Goldens: sharded chip and message (Unicode, ASCII, 60 columns), the add form with the
  toggle off and on (Unicode, ASCII), the disabled toggle on Redis 6 and its help; the ASCII
  width-identity test gained a sharded Pub/Sub fixture. Unit tests for the form and kind rules, the
  gate, and the version-keeping re-arm. Integration (`cluster_sharded_pubsub`): delivery and
  placement on the owner only, classic and sharded do not cross, delivery continues after
  `CLUSTER FAILOVER` of the owner (and the subscription is on the new owner, not the old), leaving the
  view leaves `SHARDNUMSUB` 0 on every node, and a mid-session add/remove reaches the owner. On one
  Redis 7 node sharded works beside a classic channel of the same name, and a Redis 6.2 container
  proves the gate. With the watcher disabled, the failover test fails ("no matching message within
  60s"), so it tests the watcher and not luck.
