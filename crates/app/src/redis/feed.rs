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

use fred::interfaces::ClientLike;
use fred::prelude::Client;
use fred::types::config::{Config, Server, ServerConfig};
use futures::StreamExt;
use redis_pane_core::Msg;
use redis_pane_core::clock::Clock;
use redis_pane_core::command::FeedToken;
use redis_pane_core::resolve::Credentials;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::ConnectError;

/// Which stream a feed connection carries.
///
/// `Subscribe` is deliberately not modelled yet — Pub/Sub's own task can widen
/// this (`Subscribe`/`Unsubscribe`/`PSubscribe` as the reader edits the
/// channel list) once Pub/Sub actually needs it, rather than being designed
/// speculatively here (`docs/plans/m3-feed-connection.md`'s settled lean).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    Monitor,
}

/// A feed connection's shell-local handle. Never stored in `State` — the core
/// owns no `fred` types (ADR-0011); `crates/app/src/terminal.rs` holds this
/// the same way it holds `scan_cancel`.
pub struct FeedHandle {
    cancel: CancellationToken,
    task: JoinHandle<()>,
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
    pub fn close(self) {
        self.cancel.cancel();
        self.task.abort();
    }
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
    Ok(FeedHandle { cancel, task })
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
}
