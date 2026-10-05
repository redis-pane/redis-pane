//! Which connection holds the open key's arming (M5 task 4, ADR-0022).
//!
//! On a standalone server there is one connection, so any reconnect loses the
//! arming and every reconnect re-arms (ADR-0009). On a Cluster the arming lives
//! on **one** node's connection: the owner of the open key's slot
//! (`read::arming_pipeline` pins it there). So only that node's reconnect, or
//! that slot changing owner, loses it. This module is the shell's record of
//! who the owner is, and the two listeners that act on it.
//!
//! The core is told nothing about nodes. Both listeners speak the one message
//! a reconnect always did, `Msg::Connected`, which drops the liveness claim and
//! refetches, and the refetch re-arms (ADR-0006). A node that does *not* hold
//! the arming reconnecting is not an event for the Viewer at all, but it still
//! needs `CLIENT TRACKING ON` again — a fresh connection has none, and a later
//! arming (or a slot moving onto that node) would fail without it — so that
//! listener re-probes tracking and says nothing else.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fred::interfaces::{ClusterInterface, EventInterface};
use fred::prelude::*;
use fred::types::config::Server;
use redis_pane_core::Msg;
use redis_pane_core::clock::Clock;
use tokio::sync::mpsc;

/// The open key's slot and the node whose connection holds its arming.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Armed {
    slot: u16,
    owner: Server,
}

/// The shell's record of the owner of the key that was last armed. Cheap to
/// clone; every listener shares one.
#[derive(Debug, Clone, Default)]
pub struct OpenOwner(Arc<Mutex<Option<Armed>>>);

impl OpenOwner {
    /// Record that `key` was just armed. Called after the read that armed it
    /// completed, so the routing table is as `fred` last corrected it. On a
    /// client that is not a Cluster there is no owner to remember.
    pub fn armed(&self, client: &Client, key: &[u8]) {
        let record = if client.is_clustered() {
            let slot = fred::util::redis_keyslot(key);
            crate::redis::slot_owner(client, slot).map(|owner| Armed { slot, owner })
        } else {
            None
        };
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = record;
    }

    /// Forget the record, e.g. because the client it described was replaced.
    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The node recorded as holding the arming, if any.
    pub fn owner(&self) -> Option<Server> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|a| a.owner.clone())
    }

    fn get(&self) -> Option<Armed> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Does a reconnect of `server` concern the open key? Always, on anything
    /// but a Cluster. On a Cluster, only when it is the owner.
    fn concerns(&self, client: &Client, server: &Server) -> bool {
        !client.is_clustered() || self.get().is_some_and(|a| &a.owner == server)
    }
}

/// Listen for reconnects (ADR-0009). The owner's, or any on a non-Cluster,
/// drops the liveness claim: re-probe, report the server's condition, then
/// `Msg::Connected`, whose refetch re-arms. Any other node's is probed for
/// tracking and otherwise ignored.
pub fn watch_reconnects(
    client: &Client,
    owner: &OpenOwner,
    tx: &mpsc::Sender<Msg>,
    clock: &Arc<dyn Clock>,
) {
    let mut reconnects = client.reconnect_rx();
    let probe = client.clone();
    let (owner, tx, clock) = (owner.clone(), tx.clone(), clock.clone());
    tokio::spawn(async move {
        while let Ok(server) = reconnects.recv().await {
            // A fresh connection tracks nothing, so capability must be
            // re-probed rather than remembered.
            let tracking = crate::redis::probe_tracking(&probe).await;
            if !owner.concerns(&probe, &server) {
                // Not the connection holding the arming: nothing to refetch.
                // A server that now refuses tracking is the exception, and
                // the way to say so is the same message, which degrades the
                // header visibly instead of leaving it claiming live.
                if !tracking
                    && tx
                        .send(Msg::Connected {
                            version: String::new(),
                            tracking_supported: false,
                        })
                        .await
                        .is_err()
                {
                    return;
                }
                continue;
            }
            // A refused INFO here must be as visible as any other failure —
            // defaulting to "not a replica" is the silent failure ADR-0009
            // exists to prevent (review M1).
            match crate::redis::server_conditions(&probe).await {
                Ok((read_only, condition)) => {
                    let _ = tx
                        .send(Msg::ServerState {
                            read_only,
                            condition,
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Msg::Failed {
                            command: "INFO (replica and write-rejection checks)".into(),
                            detail: e.details().to_string(),
                            at_ms: clock.now_epoch_ms(),
                        })
                        .await;
                }
            }
            if tx
                .send(Msg::Connected {
                    version: String::new(),
                    tracking_supported: tracking,
                })
                .await
                .is_err()
            {
                return;
            }
        }
    });
}

/// Is this read failure the sign of a Cluster client that has stopped
/// working, rather than of one key failing?
///
/// `fred` 10.1.0 can wedge permanently on a redirect it cannot resolve: a
/// read of a key while its slot is being migrated (`ASK`) fails with a `Routing`
/// error ("Max attempts reached"), after which *every* command on that client,
/// `sync_cluster` included, times out, even once the migration has finished.
/// Only a new client recovers, and the shell's redial (ADR-0009) makes one, so
/// a read that fails this way is reported as a lost link as well as a failed
/// read. Never true off a Cluster, where a timeout is just the command's own.
pub fn is_wedged(client: &Client, error: &Error) -> bool {
    client.is_clustered()
        && matches!(
            error.kind(),
            fred::error::ErrorKind::Routing | fred::error::ErrorKind::Timeout
        )
}

/// Listen for wire errors. A connection-level error (`IO`, or `Timeout`) means
/// the link is gone, and anything else is a command failure that belongs in a
/// notification.
///
/// Both kinds matched are what `connect_with`'s bounded-responsiveness config
/// (`crates/app/src/redis/mod.rs`) can produce for a silently dead socket: `IO`
/// from fred's own unresponsive-connection detector, and `Timeout` from the
/// per-command `default_command_timeout` backstop. The latter is ordinarily
/// returned straight to the awaiting caller rather than broadcast here, but
/// treating it the same on the rare path where it does arrive costs nothing:
/// a command that just timed out on a wire this deliberately slow to declare
/// unresponsive is not a connection worth still calling live.
///
/// On a Cluster this is deliberately *not* narrowed to the owner (ADR-0022):
/// the client has no reconnect policy, so once any node's connection drops,
/// `fred` does not redial it, its router stops serving every node (even
/// `sync_cluster` times out), and the redial is the only recovery. Adding a
/// policy so one node could heal in place was tried and rejected: with a node
/// down, `fred` 10.1.0's router retries buffered commands in a loop that never
/// yields, which starves the runtime in tests and burns a core in the app.
pub fn watch_errors(client: &Client, tx: &mpsc::Sender<Msg>, clock: &Arc<dyn Clock>) {
    let mut errors = client.error_rx();
    let (tx, clock) = (tx.clone(), clock.clone());
    tokio::spawn(async move {
        while let Ok((error, _server)) = errors.recv().await {
            let msg = if matches!(
                error.kind(),
                fred::error::ErrorKind::IO | fred::error::ErrorKind::Timeout
            ) {
                Msg::ConnectionLost
            } else {
                Msg::Failed {
                    command: "connection".into(),
                    detail: error.details().to_string(),
                    at_ms: clock.now_epoch_ms(),
                }
            };
            if tx.send(msg).await.is_err() {
                return;
            }
        }
    });
}

/// How often a Cluster client is asked to refresh its routing table while a
/// key is armed. `fred` learns of a failover or a slot move only from traffic
/// that reaches it, and an idle Viewer sends none, so without this the header
/// could say live for as long as the Viewer is left alone.
const SYNC_EVERY: Duration = Duration::from_secs(1);

/// Listen for the Cluster changing shape (a failover, a slot migration). When
/// the open key's slot has a new owner the arming is on a connection that no
/// longer sees the key, so send what a reconnect sends: the core drops the
/// liveness claim and refetches, and the refetch arms the new owner.
///
/// Nothing is sent while the owner is unchanged, so an unrelated node being
/// added or removed does not refetch the open key.
pub fn watch_topology(client: &Client, owner: &OpenOwner, tx: &mpsc::Sender<Msg>) {
    if !client.is_clustered() {
        return;
    }
    let mut changes = client.cluster_change_rx();
    let (client, owner, tx) = (client.clone(), owner.clone(), tx.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SYNC_EVERY);
        loop {
            tokio::select! {
                change = changes.recv() => match change {
                    Ok(_) => {}
                    // Lagging only means a change was missed: look anyway.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return,
                },
                _ = tick.tick() => {
                    // A failed sync is ignored on purpose: the same trouble
                    // fails the next read, which names it (R7.4).
                    if owner.get().is_some() {
                        let _ = client.sync_cluster().await;
                    }
                    continue;
                }
            }
            let Some(armed) = owner.get() else { continue };
            let now = crate::redis::slot_owner(&client, armed.slot);
            if now.is_some_and(|now| now != armed.owner) {
                let tracking = crate::redis::probe_tracking(&client).await;
                if tx
                    .send(Msg::Connected {
                        version: String::new(),
                        tracking_supported: tracking,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    });
}
