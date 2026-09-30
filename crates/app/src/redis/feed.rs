//! Second-connection plumbing for push/poll feeds — `MONITOR` today, Pub/Sub
//! later (`docs/plans/m3-feed-connection.md`, M3 task 2 phase A).
//!
//! Every read and write built through M0–M2 shares one [`fred::prelude::Client`]
//! (`crate::redis::connect_with`), matching ADR-0005's one-Connection-per-
//! process rule and the `CLIENT TRACKING` arming that connection carries for
//! the Open key (ADR-0006). `MONITOR` puts a connection into a mode that
//! streams every command the server executes and accepts nothing else until
//! the connection closes — issuing it on the main connection would freeze
//! every read and write for as long as the view stays open. So Monitor (and,
//! later, Pub/Sub) gets a second connection, opened only while its view is
//! open and closed the moment it is not.
//!
//! **This does not change ADR-0005.** One Connection per process still means
//! one *logical* target, resolved once at launch. A feed connection is a
//! second TCP socket to the *same* resolved target — transport plumbing
//! behind a Viewer, never a second Connection the title bar would need to
//! describe.

use std::sync::Arc;

use fred::clients::SubscriberClient;
use fred::interfaces::{ClientLike, EventInterface, PubsubInterface};
use fred::prelude::Client;
use fred::types::config::{Config, Server, ServerConfig};
use futures::StreamExt;
use redis_pane_core::Msg;
use redis_pane_core::clock::Clock;
use redis_pane_core::command::FeedToken;
use redis_pane_core::resolve::Credentials;
use redis_pane_core::state::Subscription;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::ConnectError;

/// Which stream a feed connection carries.
///
/// `Subscribe` carries the subscription list to dial with
/// (`docs/plans/m3-pubsub.md` phase A, M3 task 5) — the list is fixed for
/// the life of the connection *at dial time*; changing it afterwards goes
/// through [`update_subscription`] on the same connection, never a new
/// [`open_feed`] call (tearing the connection down to add a channel would
/// drop in-flight messages on every channel already subscribed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedKind {
    Monitor,
    Subscribe(Vec<Subscription>),
}

/// A feed connection's shell-local handle. Never stored in `State` — the core
/// owns no `fred` types (ADR-0011); `crates/app/src/terminal.rs` holds this
/// the same way it holds `scan_cancel`.
pub struct FeedHandle {
    cancel: CancellationToken,
    task: JoinHandle<()>,
    /// Set only for `FeedKind::Subscribe` (`m3-pubsub.md` decision 5:
    /// "closing sends `QUIT`") — `Monitor`'s `monitor::run` hands back no
    /// client to quit at all (see `open_monitor`'s doc comment), so this is
    /// `None` there. Also how [`FeedHandle::update_subscription`] reaches
    /// the live connection to issue `SUBSCRIBE`/`PSUBSCRIBE`/`UNSUBSCRIBE`/
    /// `PUNSUBSCRIBE` without tearing it down.
    subscriber: Option<SubscriberClient>,
}

impl FeedHandle {
    /// Tear the feed down. `Command::CancelScan`'s sibling
    /// (`docs/plans/m3-feed-connection.md`): `token.cancel(); drop(...)`,
    /// nothing cleverer — the read loop notices the cancellation at its next
    /// `select` and exits on its own; this is not awaited, because awaiting
    /// I/O here would put it back on the render loop's dispatch (CLAUDE.md:
    /// "the render loop never does I/O").
    ///
    /// For `FeedKind::Monitor` specifically, see `open_monitor`'s doc comment
    /// for the one real limitation this leaves: the socket itself closes when
    /// fred's own internal forwarding task next tries to send a line, not
    /// synchronously with this call.
    ///
    /// For `FeedKind::Subscribe`, a real client is on hand, so this also
    /// sends `QUIT` — spawned as its own detached task rather than awaited,
    /// for the same "never do I/O on the render loop's dispatch" reason.
    /// Redis unsubscribes everything implicitly on disconnect regardless
    /// (`docs/plans/m3-pubsub.md`'s "unsubscribing cleanly" section), so a
    /// `QUIT` that never lands (process killed before the spawned task
    /// runs) still leaves no orphaned subscription — this is belt-and-
    /// suspenders tidiness, not the mechanism the no-orphan guarantee
    /// actually rests on.
    pub fn close(self) {
        self.cancel.cancel();
        self.task.abort();
        if let Some(client) = self.subscriber {
            tokio::spawn(async move {
                let _ = client.quit().await;
            });
        }
    }

    /// `Command::UpdateSubscription`: add and/or remove subscriptions on
    /// this connection without closing it (`m3-pubsub.md` phase A). A no-op
    /// — not an error — for a `Monitor` handle (`subscriber: None`):
    /// `terminal.rs` only ever calls this while the open feed is Pub/Sub's,
    /// but nothing here assumes that rather than checking.
    ///
    /// A failing `SUBSCRIBE`/`PSUBSCRIBE`/`UNSUBSCRIBE`/`PUNSUBSCRIBE` (an
    /// ACL's `-NOPERM` on a restricted channel, most plausibly) is never a
    /// silently dropped `Err` — CLAUDE.md names exactly that defect ("a bare
    /// `Err(_) => return` inside a task"). Each of the up to four underlying
    /// commands is awaited and, on failure, reported through `tx` as
    /// `Msg::SubscriptionFailed`, which both raises the R7.4 notification
    /// (naming the failing command) and — for a failed **add** specifically
    /// — tells the core to drop that chip rather than leave it showing as
    /// subscribed when the server never actually subscribed it.
    pub fn update_subscription(
        &self,
        add: Vec<Subscription>,
        remove: Vec<Subscription>,
        tx: Sender<Msg>,
        clock: Arc<dyn Clock>,
    ) {
        let Some(client) = self.subscriber.clone() else {
            return;
        };
        tokio::spawn(async move {
            let (add_channels, add_patterns) = split_subscriptions(&add);
            let (remove_channels, remove_patterns) = split_subscriptions(&remove);

            if !add_channels.is_empty()
                && let Err(e) = client.subscribe(add_channels).await
            {
                let failed = subs_where(&add, false);
                report_subscription_failure(&tx, &clock, "SUBSCRIBE", &e, failed).await;
            }
            if !add_patterns.is_empty()
                && let Err(e) = client.psubscribe(add_patterns).await
            {
                let failed = subs_where(&add, true);
                report_subscription_failure(&tx, &clock, "PSUBSCRIBE", &e, failed).await;
            }
            if !remove_channels.is_empty()
                && let Err(e) = client.unsubscribe(remove_channels).await
            {
                let failed = subs_where(&remove, false);
                report_subscription_failure(&tx, &clock, "UNSUBSCRIBE", &e, failed).await;
            }
            if !remove_patterns.is_empty()
                && let Err(e) = client.punsubscribe(remove_patterns).await
            {
                let failed = subs_where(&remove, true);
                report_subscription_failure(&tx, &clock, "PUNSUBSCRIBE", &e, failed).await;
            }
            // The same synchronizing round trip `open_subscribe` does after
            // its own initial SUBSCRIBE/PSUBSCRIBE, and for the identical
            // reason: a successful `add` here must be visible to another
            // connection's `PUBLISH` before this call returns, or a fast
            // publisher racing the reader's `a add` keystroke could be
            // missed with nothing to say so.
            if !add.is_empty() {
                let _: Result<String, _> = client.ping(None).await;
            }
        });
    }
}

/// The entries of `subs` that are patterns (`is_pattern == true`) or
/// channels (`is_pattern == false`) — used to report exactly which
/// [`Subscription`]s a failed `SUBSCRIBE`/`PSUBSCRIBE`/`UNSUBSCRIBE`/
/// `PUNSUBSCRIBE` applied to, since [`split_subscriptions`] already threw
/// that distinction away to build the plain-string argument lists the fred
/// call itself wants.
fn subs_where(subs: &[Subscription], is_pattern: bool) -> Vec<Subscription> {
    subs.iter()
        .filter(|s| s.is_pattern() == is_pattern)
        .cloned()
        .collect()
}

/// Send `Msg::SubscriptionFailed` for a failed subscription command — the
/// one place [`FeedHandle::update_subscription`] turns a fred `Err` into the
/// R7.4 notification (CLAUDE.md: never a silently dropped `Err`).
async fn report_subscription_failure(
    tx: &Sender<Msg>,
    clock: &Arc<dyn Clock>,
    command: &str,
    error: &fred::error::Error,
    subs: Vec<Subscription>,
) {
    let at_ms = clock.now_epoch_ms();
    let _ = tx
        .send(Msg::SubscriptionFailed {
            command: command.to_string(),
            detail: super::describe(error),
            at_ms,
            subs,
        })
        .await;
}

/// Split a subscription list into channel names and pattern names, in the
/// shape `SUBSCRIBE`/`PSUBSCRIBE` (or their `UN`-prefixed counterparts) want.
fn split_subscriptions(subs: &[Subscription]) -> (Vec<String>, Vec<String>) {
    let mut channels = Vec::new();
    let mut patterns = Vec::new();
    for sub in subs {
        if sub.is_pattern() {
            patterns.push(sub.name().to_string());
        } else {
            channels.push(sub.name().to_string());
        }
    }
    (channels, patterns)
}

/// Dial a second connection to the same resolved target [`super::connect_with`]
/// already resolves — same URL, same credential path
/// ([`super::build_config`]), never the credential-less [`super::connect`]. A
/// feed against a password-protected server must authenticate exactly the way
/// the main connection does, not silently connect unauthenticated and fail
/// obscurely on the first line.
///
/// Streams translated [`Msg`]s into `tx`, the same channel every other `Msg`
/// in the app arrives on — a feed connection is not a second source of truth
/// about the app's own state, only another producer of events into the loop
/// that already exists.
///
/// `main` is the app's own main connection — used only to ask fred what
/// server it is actually talking to right now (`main.active_connections()`)
/// when the resolved target is behind Sentinel (ADR-0008: Sentinel is a v1
/// target). See [`open_monitor`]'s doc comment for why that lookup exists.
pub async fn open_feed(
    url: &str,
    credentials: &Credentials,
    kind: FeedKind,
    token: FeedToken,
    tx: Sender<Msg>,
    clock: Arc<dyn Clock>,
    main: &Client,
) -> Result<FeedHandle, ConnectError> {
    match kind {
        FeedKind::Monitor => open_monitor(url, credentials, token, tx, clock, main).await,
        FeedKind::Subscribe(subs) => open_subscribe(url, credentials, subs, token, tx, clock).await,
    }
}

/// # Why `fred::monitor::run`, not a hand-rolled socket
///
/// `fred` 10 does not put `MONITOR` on `Client` as an ordinary command —
/// the crate wires `CommandKind::Monitor` only inside its own `monitor`
/// module, gated by the `monitor` Cargo feature (this crate now enables it;
/// it pulls in `nom` to parse the `MONITOR` line format the same way
/// `redis-cli` does). `fred::monitor::run(config)` dials its own connection
/// from a `Config`, sends `MONITOR`, and returns a
/// `Stream<Item = fred::monitor::MonitorCommand>` with the timestamp, db,
/// client address and command already parsed. That is exactly this task's
/// shape, and it is proof against a hand-rolled parser drifting from a wire
/// format Redis has changed before — preferring it over reading the raw
/// socket ourselves is the "prefer fred's supported API" call from
/// `docs/plans/m3-feed-connection.md`.
///
/// The cost, worth recording rather than discovering later: `monitor::run`
/// only supports `ServerConfig::Centralized` (this app's Cluster-is-not-v1
/// scope already matches, ADR-0008), and — the real limitation — it hands
/// back a bare `Stream`, no `Client`, no connection handle. There is nothing
/// to `.quit()` or drop to close the socket synchronously. Internally, fred
/// spawns its own task that reads `MONITOR` lines and forwards them over an
/// `mpsc` channel; it discovers our end is gone only when it next tries to
/// forward a line and the send fails, at which point *it* drops the
/// connection. Dropping our `Stream` (which this module does on
/// cancellation) therefore stops **our** consumption immediately, but the
/// TCP socket itself lingers until the next line arrives to be forwarded —
/// in practice fast, since the app's own traffic and anything else on the
/// server keeps lines arriving, but not a guaranteed-synchronous close the
/// way dropping a `fred::Client` is. A silent, wire-level drop (the socket
/// simply going dead, no `MONITOR` line and no error ever arriving) is
/// likewise only noticed once the server manages to say *something* over
/// it — `fred::monitor::run` sets no unresponsive-connection timeout of its
/// own the way `connect_with` does for the main connection. The scenario
/// this phase's integration tests exercise (`CLIENT KILL`, which closes the
/// socket immediately) is unaffected: that surfaces as an ordinary EOF/error
/// on the read, not a black hole.
///
/// # Sentinel (ADR-0008)
///
/// `monitor::run`'s own `ServerConfig` match (`fred-10.1.0/src/monitor/utils.rs`)
/// is narrower still than the paragraph above suggests: it accepts
/// `ServerConfig::Centralized` only, and returns `Err(Config, "Expected
/// centralized server config.")` for anything else — Sentinel included, even
/// though Sentinel is a v1 target here, not a deferred one (Cluster is the
/// deferred one). Left alone, a Sentinel-resolved target would still *fail
/// clearly* (that `Err` becomes an ordinary `ConnectError::Unreachable`,
/// reported the same way any other feed-open failure is — never a hang), but
/// the message would be fred's internal jargon and Monitor would simply not
/// work against a Sentinel Connection, which is a real gap for a target this
/// app is supposed to support.
///
/// So: when `build_config` produced a Sentinel `ServerConfig`, this asks the
/// *main* connection — which has already done Sentinel discovery and is
/// talking to the primary right now — for that address
/// (`main.active_connections()`, a synchronous, no-I/O read of `fred`'s own
/// cached routing) and rewrites the feed's `Config` to `Centralized` against
/// it, keeping every other field (password, username, TLS) exactly as
/// `build_config` resolved them. This is deliberately not Sentinel discovery
/// of its own for a second, short-lived connection — the main connection
/// already paid that cost and already knows the answer. A Clustered target
/// is refused outright with a reason that says why, rather than reaching
/// `monitor::run`'s own generic error: Cluster is not a v1 target at all
/// (ADR-0008), so this is "not supported yet", not "temporarily
/// unreachable".
async fn open_monitor(
    url: &str,
    credentials: &Credentials,
    token: FeedToken,
    tx: Sender<Msg>,
    clock: Arc<dyn Clock>,
    main: &Client,
) -> Result<FeedHandle, ConnectError> {
    let config: Config = super::build_config(url, credentials)?;
    let config = monitor_config(config, &main.active_connections())?;
    let stream = fred::monitor::run(config)
        .await
        .map_err(|e| ConnectError::Unreachable(super::describe(&e)))?;

    let cancel = CancellationToken::new();
    let task = spawn_monitor_reader(stream, token, tx, clock, cancel.clone());
    Ok(FeedHandle {
        cancel,
        task,
        subscriber: None,
    })
}

/// Rewrite `config` into a shape `fred::monitor::run` will accept — see
/// [`open_monitor`]'s doc comment for why this exists at all. Pure and
/// synchronous so it can be unit-tested without a live server: `active` is
/// whatever `main.active_connections()` returned, passed in rather than
/// looked up here.
fn monitor_config(mut config: Config, active: &[Server]) -> Result<Config, ConnectError> {
    if config.server.is_clustered() {
        return Err(ConnectError::Unreachable(
            "MONITOR is not supported against a Cluster target (ADR-0008: Cluster is not a v1 target)"
                .into(),
        ));
    }
    if config.server.is_sentinel() {
        let primary = active.first().cloned().ok_or_else(|| {
            ConnectError::Unreachable(
                "could not find the Sentinel-resolved primary's address on the main connection"
                    .into(),
            )
        })?;
        config.server = ServerConfig::Centralized { server: primary };
    }
    Ok(config)
}

/// The read loop: translate every `MONITOR` line into `Msg::MonitorLine` and
/// send it in, until cancelled or the stream ends on its own.
///
/// A silent drop — the server closing the connection, `CLIENT KILL`, a
/// network reset — ends the underlying `Stream` (`next()` returns `None`)
/// rather than hanging: `forward_results`'s inner loop (`fred`'s own code)
/// exits its `while let Some(frame) = framed.next().await` the moment the
/// socket read itself ends, and `rx.into_stream()` ends with it. That is what
/// turns a dead feed connection into `Msg::FeedClosed` here, never a stuck
/// `select` — the same shape `Msg::ConnectionLost` already gives the main
/// connection's watcher (`crates/app/src/terminal.rs`'s `watch_link`,
/// `crates/core/src/update/link.rs`).
fn spawn_monitor_reader(
    mut stream: impl futures::Stream<Item = fred::monitor::MonitorCommand> + Unpin + Send + 'static,
    token: FeedToken,
    tx: Sender<Msg>,
    clock: Arc<dyn Clock>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                biased;
                // Checked first so a reader-initiated close (`Esc`,
                // `Command::CloseFeed`) is answered at the next loop
                // iteration rather than waiting for another line to arrive —
                // the same ordering `crate::redis::scan::stream_keys` uses
                // for its own cancellation check. Cancellation sends nothing
                // back: the reader already knows they closed it (the core set
                // that in the same step it emitted `Command::CloseFeed`), so
                // this task's job is only to stop, not to narrate a close
                // nobody is waiting to hear about.
                () = cancel.cancelled() => return,
                item = stream.next() => item,
            };
            match next {
                Some(command) => {
                    let at_ms = clock.now_epoch_ms();
                    // `MonitorCommand`'s `Display` prints the same format
                    // `redis-cli` does — the server's own timestamp, db,
                    // client and command — which is the raw text this phase
                    // hands the core; parsing it into columns is Monitor's
                    // own job (`docs/plans/m3-monitor.md`, phase B).
                    let raw = command.to_string();
                    if tx
                        .send(Msg::MonitorLine { token, at_ms, raw })
                        .await
                        .is_err()
                    {
                        return; // the UI is gone
                    }
                }
                None => {
                    // The stream ended on its own: the server closed the
                    // connection (`CLIENT KILL`, a restart) or the network
                    // dropped it. Never a hang — this is the one signal that
                    // turns a dead feed connection into `Msg::FeedClosed`,
                    // mirroring `Msg::ConnectionLost` for the main connection.
                    let _ = tx
                        .send(Msg::FeedClosed {
                            token,
                            reason: Some("the feed connection closed".into()),
                        })
                        .await;
                    return;
                }
            }
        }
    })
}

/// Dial a `SubscriberClient` (`docs/plans/m3-pubsub.md` decision 5, M3 task
/// 5 phase A) and subscribe it to `subs` before ever handing back a handle —
/// decision 6's lazy-open only reaches this function with a non-empty list
/// in the first place, but an empty list is still handled correctly (dials,
/// subscribes to nothing, and sits there — `Command::UpdateSubscription`
/// then adds the first one on the same connection).
///
/// Unlike [`open_monitor`], this needs no Sentinel-primary rewrite: a
/// `SubscriberClient` is an ordinary client built from [`super::build_config`],
/// and fred's own Sentinel support works for it exactly as it does for the
/// main connection (decision 5: "a normal client, so Sentinel works as
/// is"). **No reconnect policy** is set here — `Builder::from_config`
/// defaults to `policy: None`, and nothing in this function calls
/// `set_policy`/`with_reconnect_policy` — so a drop surfaces as this task's
/// stream ending, never a silent auto-reconnect that would leave `State`
/// believing the feed is still the one it subscribed.
async fn open_subscribe(
    url: &str,
    credentials: &Credentials,
    subs: Vec<Subscription>,
    token: FeedToken,
    tx: Sender<Msg>,
    clock: Arc<dyn Clock>,
) -> Result<FeedHandle, ConnectError> {
    let config = super::build_config(url, credentials)?;
    let mut builder = fred::types::Builder::from_config(config);
    // Same backstop `connect_with` gives the main connection, and for the
    // same reason: fred's default `default_command_timeout` is `0` (never),
    // and a `SUBSCRIBE`/`PSUBSCRIBE` an ACL denies is not merely slow — its
    // error reply does not resolve fred's own "waiting for a subscribe
    // confirmation" bookkeeping the way an ordinary command's reply
    // resolves an ordinary await, so without this the call inside
    // `FeedHandle::update_subscription`/this function's own initial
    // subscribe hangs forever rather than erroring, which is exactly the
    // silent-failure shape M3 task 5 (b) exists to rule out. Generous
    // rather than snappy, matching `connect_with`'s own reasoning (this
    // product runs over SSH).
    builder.with_performance_config(|performance| {
        performance.default_command_timeout = std::time::Duration::from_secs(10);
    });
    let client = builder
        .build_subscriber_client()
        .map_err(|e| ConnectError::Unreachable(super::describe(&e)))?;
    client
        .init()
        .await
        .map_err(|e| ConnectError::Unreachable(super::describe(&e)))?;

    // Captured *before* issuing SUBSCRIBE/PSUBSCRIBE, deliberately: fred's
    // `message_rx()` subscribes to a `tokio::broadcast` channel that already
    // exists on the client (created at `init`, not per-subscription — see
    // `EventInterface::message_rx`'s own body, `notifications.pubsub.load()
    // .subscribe()`), and a broadcast receiver only ever sees messages sent
    // *after* it was created. Capturing it after the subscribe call would
    // leave a real, if narrow, window between the server confirming the
    // subscription and this receiver actually attaching — long enough, in
    // practice under load, to lose the first message a fast publisher sends
    // right after the subscribe confirmation lands. Capturing both receivers
    // first closes that window: whatever fred forwards from this point on,
    // including the SUBSCRIBE confirmation itself, this task's reader
    // (spawned below) will see once it starts polling — a `broadcast`
    // channel buffers for a receiver that exists but has not yet called
    // `.recv()`, it just does not buffer for a receiver that does not exist
    // yet.
    let cancel = CancellationToken::new();
    // Shared once-only guard: whichever of the two watchers below notices
    // the connection is gone first (`message_rx` closing, or a
    // connection-level event on `error_rx`) is the one that actually sends
    // `Msg::FeedClosed` — never both (see `spawn_subscribe_reader`'s doc
    // comment for why there are two watchers at all).
    let reported = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let message_rx = client.message_rx();
    let error_rx = client.error_rx();

    let (channels, patterns) = split_subscriptions(&subs);
    if !channels.is_empty() {
        client
            .subscribe(channels)
            .await
            .map_err(|e| ConnectError::Unreachable(super::describe(&e)))?;
    }
    if !patterns.is_empty() {
        client
            .psubscribe(patterns)
            .await
            .map_err(|e| ConnectError::Unreachable(super::describe(&e)))?;
    }
    // A synchronizing round trip on this same connection, after every
    // SUBSCRIBE/PSUBSCRIBE above has resolved: verified against a real
    // container (M3 task 5) that fred's own `subscribe()`/`psubscribe()`
    // await can resolve measurably before the server's own subscription
    // state is visible to another connection's `PUBLISH` — a fast publisher
    // right after `open_feed` returns could otherwise see zero receivers
    // and this feed would simply never observe that message, no error, no
    // `Msg::FeedClosed`, nothing. Redis processes commands from one
    // connection strictly in the order they were sent, so a `PING`'s reply
    // on *this* connection cannot arrive before every command sent before
    // it (the SUBSCRIBE(s)) has been fully applied server-side — unlike
    // `subscribe()`'s own await, which this evidence says is not a reliable
    // signal of that on its own. Cheap (one extra round trip, once per
    // dial/`Command::UpdateSubscription` call, not per message) and cheaper
    // than the alternative of a reader who opens Pub/Sub and misses
    // whatever was published in the first instant.
    if !subs.is_empty() {
        let _: Result<String, _> = client.ping(None).await;
    }

    let task = spawn_subscribe_reader(
        message_rx,
        subs,
        token,
        tx.clone(),
        clock,
        cancel.clone(),
        reported.clone(),
    );
    spawn_subscribe_error_watcher(error_rx, token, tx, cancel.clone(), reported);
    Ok(FeedHandle {
        cancel,
        task,
        subscriber: Some(client),
    })
}

/// The read loop: translate every arriving Pub/Sub message into
/// `Msg::PubSubMessage`, until cancelled, a connection-level error arrives,
/// or the broadcast channel closes.
///
/// `subs` is a snapshot of the subscription list *as dialed* — it is used
/// only to work out `via` for a `pmessage` (see [`matched_pattern`]'s doc
/// comment for why fred's own `Message` cannot answer that question
/// directly); it is never mutated here, so a mid-session
/// `Command::UpdateSubscription` (`FeedHandle::update_subscription`) does
/// not update what this loop matches against. A message on a pattern added
/// after this task started therefore reports no `via` until the feed is
/// reopened — an acceptable gap for phase A (`via` is best-effort
/// attribution, not correctness-critical: the channel itself is always
/// right), worth tightening in phase B if it turns out to matter once the
/// view can actually show it.
///
/// A second watcher, [`spawn_subscribe_error_watcher`], covers the case this
/// loop alone cannot: a server-side drop (`CLIENT KILL`, a network reset)
/// with no message in flight leaves nothing for `message_rx` to fail on —
/// unlike `MONITOR`'s `fred::monitor::run`, a `SubscriberClient`'s
/// `message_rx()` broadcast sender is tied to the *client*, not the
/// connection, so it can sit open indefinitely after the socket is
/// actually gone. `reported` is the once-only guard between the two: only
/// the first of them to notice the connection is dead sends
/// `Msg::FeedClosed`.
fn spawn_subscribe_reader(
    mut message_rx: tokio::sync::broadcast::Receiver<fred::types::Message>,
    subs: Vec<Subscription>,
    token: FeedToken,
    tx: Sender<Msg>,
    clock: Arc<dyn Clock>,
    cancel: CancellationToken,
    reported: Arc<std::sync::atomic::AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                item = message_rx.recv() => item,
            };
            match next {
                Ok(message) => {
                    if message.kind == fred::types::MessageKind::SMessage {
                        // Sharded Pub/Sub is out of scope
                        // (`docs/plans/m3-pubsub.md`'s "Out of scope":
                        // `SSUBSCRIBE` is Cluster-only, and Cluster is not a
                        // v1 target, ADR-0008) — this app never `SSUBSCRIBE`s,
                        // so this should be unreachable, but a message of a
                        // kind we did not ask for is dropped rather than
                        // mis-attributed.
                        continue;
                    }
                    let at_ms = clock.now_epoch_ms();
                    let channel = message.channel.as_bytes().to_vec();
                    let via = if message.kind == fred::types::MessageKind::PMessage {
                        matched_pattern(&subs, &channel)
                    } else {
                        None
                    };
                    let payload = message.value.convert::<Vec<u8>>().unwrap_or_default();
                    if tx
                        .send(Msg::PubSubMessage {
                            token,
                            at_ms,
                            channel,
                            via,
                            payload,
                        })
                        .await
                        .is_err()
                    {
                        return; // the UI is gone
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // The broadcast channel's own buffer overflowed — fred
                    // dropped messages we were too slow to read. Not a
                    // closed feed: keep reading rather than tearing the
                    // connection down over a burst.
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    // fred drops every sender when the underlying connection
                    // is gone for good (closed by the reader's own `QUIT`,
                    // the server, or the network) — the same "stream ended,
                    // never a hang" signal `spawn_monitor_reader` turns into
                    // `Msg::FeedClosed` for `MONITOR`.
                    if !reported.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        let _ = tx
                            .send(Msg::FeedClosed {
                                token,
                                reason: Some("the feed connection closed".into()),
                            })
                            .await;
                    }
                    cancel.cancel(); // stop the error watcher too
                    return;
                }
            }
        }
    })
}

/// Watches for a connection-level error on `error_rx` — the signal
/// [`spawn_subscribe_reader`]'s doc comment explains `message_rx` alone can
/// miss (a server-side drop with nothing in flight). With no
/// `ReconnectPolicy` set (decision 5), any such error means the connection
/// is gone for good, exactly like `Msg::ConnectionLost` means for the main
/// connection (`Shell::watch_link`, `crates/app/src/terminal.rs`) — so it
/// becomes `Msg::FeedClosed` here, guarded by `reported` so the message
/// reader's own `Closed` path (if it wins the race instead) never double-
/// reports.
///
/// This task's own loop only ever awaits `error_rx.recv()` — it never spins:
/// `Lagged` re-awaits (a transient burst, not a permanent ready state), and
/// `Closed` (the client itself was dropped) simply ends the task rather than
/// looping on an always-ready future, which is exactly the shape that would
/// busy-loop if handled as another `tokio::select!` arm inside
/// [`spawn_subscribe_reader`]'s own loop instead of its own task.
fn spawn_subscribe_error_watcher(
    mut error_rx: tokio::sync::broadcast::Receiver<(
        fred::error::Error,
        Option<fred::types::config::Server>,
    )>,
    token: FeedToken,
    tx: Sender<Msg>,
    cancel: CancellationToken,
    reported: Arc<std::sync::atomic::AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return,
                result = error_rx.recv() => match result {
                    Ok((error, _server)) => {
                        if !reported.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            let _ = tx
                                .send(Msg::FeedClosed {
                                    token,
                                    reason: Some(super::describe(&error)),
                                })
                                .await;
                        }
                        cancel.cancel(); // stop the message reader too
                        return;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
            }
        }
    })
}

/// Which of `subs`'s patterns matched `channel`, for a `pmessage`.
///
/// fred's own `Message` does not carry the pattern text that actually
/// matched (`frame_to_pubsub` in fred 10.1.0 parses and then discards it —
/// see this module's `open_subscribe` call site for the full account); the
/// only way to answer "`via` which pattern" from the public API is to test
/// `channel` against our own subscribed patterns and report the first that
/// matches, using [`redis_pane_core::state::redis_glob_match`] — the exact
/// Redis glob semantics, not this crate's own looser display-filter glob.
///
/// **Genuinely ambiguous, not just approximate, when two subscribed
/// patterns can both match the same channel** (e.g. `user:*` and `user:4?`):
/// Redis delivers one `pmessage` *per matching pattern*, so a channel that
/// matches both arrives as two separate messages with identical channel and
/// payload — and there is no way, from fred's public API, to tell either
/// delivery apart or know which pattern produced which. This reports
/// whichever subscribed pattern is checked first for *both* deliveries
/// (M3 task 5 (c)): assigning the two matches to the two patterns in
/// delivery order was considered and rejected, since Redis's internal
/// iteration order over subscribed patterns is not a documented guarantee —
/// doing that would swap a visible, documented limitation for a false
/// confidence that happens to be right only when the server's internal
/// pattern order matches this app's subscription order. The core's own
/// `PubSubMessage::ambiguous_via` flags exactly this case so the detail
/// strip can hedge its wording instead of asserting a `via` it cannot
/// actually back up (`docs/plans/m3-pubsub.md`, DESIGN §6.7).
fn matched_pattern(subs: &[Subscription], channel: &[u8]) -> Option<Vec<u8>> {
    subs.iter()
        .filter(|s| s.is_pattern())
        .find(|s| redis_pane_core::state::redis_glob_match(s.name().as_bytes(), channel))
        .map(|s| s.name().as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fred::types::config::ServerConfig;

    fn centralized(host: &str, port: u16) -> Config {
        Config {
            server: ServerConfig::new_centralized(host, port),
            ..Config::default()
        }
    }

    fn sentinel() -> Config {
        Config {
            server: ServerConfig::new_sentinel(
                vec![("sentinel.example", 26379u16)],
                "mymaster".to_string(),
            ),
            ..Config::default()
        }
    }

    fn clustered() -> Config {
        Config {
            server: ServerConfig::new_clustered(vec![("cluster.example", 6379u16)]),
            ..Config::default()
        }
    }

    #[test]
    fn a_centralized_config_passes_through_unchanged() {
        let config = centralized("db.example", 6379);
        let out = monitor_config(config.clone(), &[]).unwrap();
        assert_eq!(out.server, config.server);
    }

    #[test]
    fn a_sentinel_config_is_rewritten_to_centralized_against_the_active_primary() {
        let primary = Server::new("10.0.0.5", 6379);
        let out = monitor_config(sentinel(), std::slice::from_ref(&primary)).unwrap();
        assert_eq!(out.server, ServerConfig::Centralized { server: primary });
    }

    #[test]
    fn a_sentinel_config_with_no_active_connection_known_refuses_clearly() {
        let err = monitor_config(sentinel(), &[]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Sentinel"), "{msg}");
    }

    #[test]
    fn a_clustered_config_is_refused_before_ever_reaching_fred() {
        let primary = Server::new("10.0.0.5", 6379);
        let err = monitor_config(clustered(), std::slice::from_ref(&primary)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Cluster"), "{msg}");
    }

    // The glob matcher itself now lives in `redis_pane_core::state`
    // (`redis_glob_match`) and is tested there — this module only tests
    // `matched_pattern`'s use of it.
    #[test]
    fn matched_pattern_reports_the_subscribed_pattern_a_channel_matched() {
        let subs = vec![
            Subscription::Channel("orders".into()),
            Subscription::Pattern("user:*".into()),
        ];
        assert_eq!(matched_pattern(&subs, b"user:42"), Some(b"user:*".to_vec()));
        assert_eq!(matched_pattern(&subs, b"orders"), None);
        assert_eq!(matched_pattern(&subs, b"unrelated"), None);
    }
}
