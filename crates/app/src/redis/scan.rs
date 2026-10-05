//! The keyspace source: a stream of keys with progress (R2.1, PLAN M1.2).
//!
//! **This abstracts over a stream of keys, not over a cursor.** v1 has exactly
//! one cursor behind it, a Cluster has N, one per primary, merged here
//! (ADR-0008, M5 task 3). Nothing above this module may learn
//! how many there are — that is the whole reason the abstraction exists.
//!
//! `SCAN` only, never `KEYS`: cursor-based, streaming, resumable, and
//! cancellable at any page boundary.

use std::time::Duration;

use fred::prelude::*;
use fred::types::scan::Scanner;
use futures::StreamExt;
use redis_pane_core::Msg;
use redis_pane_core::state::InterruptReason;
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// How many keys to ask for per page.
///
/// `COUNT` is a hint, not a guarantee. Small enough that a page arrives
/// promptly and the list fills visibly; large enough not to pay a round trip
/// per handful of keys.
const PAGE: u32 = 500;

/// How long one primary may go without answering a page before the scan fails
/// naming it. `fred` never reports a dead node's `SCAN` as an error (it
/// retries silently), so without this a scan would hang with no message.
const NODE_PAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// How often a running Cluster scan re-reads the routing table. `fred` only
/// notices a topology change when traffic reaches it, and the scan's own
/// traffic goes through per-node connections, so this is what makes
/// `cluster_change_rx` fire during a scan with nothing else going on.
const TOPOLOGY_SYNC_EVERY: Duration = Duration::from_secs(1);

/// The knobs of one scan. Production uses [`Tuning::default`]; the integration
/// suite shrinks them so a failover or a dead node can be provoked
/// deterministically.
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub page_size: u32,
    pub node_page_timeout: Duration,
    pub topology_sync_every: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            page_size: PAGE,
            node_page_timeout: NODE_PAGE_TIMEOUT,
            topology_sync_every: TOPOLOGY_SYNC_EVERY,
        }
    }
}

type Changes = tokio::sync::broadcast::Receiver<Vec<fred::types::ClusterStateChange>>;

/// What a node's scanning task reports to the page loop.
enum NodeEvent {
    Keys(Vec<Vec<u8>>),
    /// This node's cursor is exhausted.
    Done,
    Failed(String),
}

/// Traverse the keyspace, sending batches as they arrive.
///
/// One function for both deployments (M5 task 3): a clustered client walks
/// every primary, anything else walks its one cursor. Either way the keyspace
/// is N node scans merged into one stream, the page loop, cancellation and
/// messages are shared, and the core sees only `Msg::ScanBatch`.
///
/// Returns when the traversal finishes, fails, is interrupted, or is
/// cancelled — having sent a terminal [`Msg`] in each case, so the core always
/// learns how it ended.
pub async fn stream_keys(
    client: &Client,
    pattern: Option<&str>,
    tx: Sender<Msg>,
    cancel: CancellationToken,
) {
    stream_keys_with(client, pattern, tx, cancel, Tuning::default()).await;
}

/// [`stream_keys`] with the [`Tuning`] chosen by the caller.
pub async fn stream_keys_with(
    client: &Client,
    pattern: Option<&str>,
    tx: Sender<Msg>,
    cancel: CancellationToken,
    tuning: Tuning,
) {
    let clustered = client.is_clustered();

    // Subscribed before anything is read, so a change between the topology
    // snapshot below and the first page cannot be missed.
    let mut changes = clustered.then(|| client.cluster_change_rx());

    // One connection per primary, opened first: the denominator is read over
    // the same connections the scan then uses, and a node that cannot answer
    // is named by the connection that failed (R7.4), not guessed at.
    let split = if clustered {
        match connect_nodes(client, tuning.node_page_timeout).await {
            Ok(split) => Some(split),
            Err(error) => {
                let _ = tx.send(Msg::ScanFailed { error }).await;
                return;
            }
        }
    } else {
        None
    };

    // DBSIZE is the denominator in "41,203 of ~180,000". It is an estimate by
    // nature — the keyspace moves while we walk it — which is why the readout
    // says `~` and never a percentage of something exact.
    let estimated_total = match &split {
        // Never a smaller total: a primary that cannot answer fails the scan,
        // with the node named.
        Some(split) => match cluster_dbsize(split, tuning.node_page_timeout).await {
            Ok(total) => total,
            Err(error) => {
                let _ = tx.send(Msg::ScanFailed { error }).await;
                close(split.iter().map(|(_, c)| c.clone()).collect()).await;
                return;
            }
        },
        None => client.dbsize().await.unwrap_or(0),
    };
    if tx.send(Msg::ScanStarted { estimated_total }).await.is_err() {
        close(split.iter().flatten().map(|(_, c)| c.clone()).collect()).await;
        return;
    }

    let pattern = pattern.unwrap_or("*").to_string();
    let mut nodes = Nodes::start(client, split, &pattern, tuning);
    let outcome = page_loop(&mut nodes, &mut changes, &tx, &cancel).await;
    if let Some(msg) = outcome {
        let _ = tx.send(msg).await;
    }
    nodes.stop().await;
}

/// The page loop, shared by both deployments. `None` means the UI is gone and
/// there is nobody to tell.
async fn page_loop(
    nodes: &mut Nodes,
    changes: &mut Option<Changes>,
    tx: &Sender<Msg>,
    cancel: &CancellationToken,
) -> Option<Msg> {
    let mut done = 0usize;
    loop {
        let event = tokio::select! {
            biased;
            // Cancellation is checked first so `Esc` is answered at the next
            // page boundary rather than after the whole keyspace.
            _ = cancel.cancelled() => return Some(Msg::ScanCancelled),
            // A topology change makes the walk's coverage unknowable: keys
            // may be missed or repeated. Say so rather than finish quietly.
            () = topology_changed(changes) => return Some(interrupted()),
            event = nodes.rx.recv() => event,
        };

        match event {
            Some(NodeEvent::Keys(batch)) => {
                if !batch.is_empty() && tx.send(Msg::ScanBatch { keys: batch }).await.is_err() {
                    return None; // the UI is gone
                }
            }
            Some(NodeEvent::Done) => done += 1,
            Some(NodeEvent::Failed(error)) => {
                // A failover surfaces as errors on the way down; the change is
                // the cause, so report that.
                return Some(if changed_now(changes) {
                    interrupted()
                } else {
                    Msg::ScanFailed { error }
                });
            }
            // "The channel closed" is the end of the scan, never a page's
            // `has_more()` — but only once every node said it was done. A
            // node task that vanished must not read as a finished keyspace.
            None if done == nodes.count => return Some(Msg::ScanComplete),
            None => {
                return Some(Msg::ScanFailed {
                    error: "a scan task stopped before its node finished".into(),
                });
            }
        }
    }
}

fn interrupted() -> Msg {
    Msg::ScanInterrupted {
        reason: InterruptReason::TopologyChanged,
    }
}

/// The scanning tasks of one traversal, one per node, merged on `rx`.
///
/// A Cluster is scanned with one connection per primary (`split_cluster`),
/// not `fred`'s `scan_cluster`. `scan_cluster` was tried first (M5 task 3,
/// decision 1) and failed the confirm-at-build test: with one primary down it
/// delivers no pages at all, not even from the healthy nodes, and never an
/// error, so a failing node could be neither reported nor named.
struct Nodes {
    rx: mpsc::Receiver<NodeEvent>,
    count: usize,
    tasks: JoinSet<()>,
    /// Connections this scan opened and must close.
    owned: Vec<Client>,
}

impl Nodes {
    fn start(
        client: &Client,
        split: Option<Vec<(String, Client)>>,
        pattern: &str,
        tuning: Tuning,
    ) -> Nodes {
        // Small: a node waits on the page loop rather than reading ahead, so
        // memory stays bounded by the Loaded set (see `page.next()` below).
        let (tx, rx) = mpsc::channel(4);
        let mut tasks = JoinSet::new();

        let Some(split) = split else {
            tasks.spawn(scan_node(
                client.clone(),
                None,
                pattern.to_string(),
                tuning.page_size,
                None,
                tx,
            ));
            return Nodes {
                rx,
                count: 1,
                tasks,
                owned: Vec::new(),
            };
        };

        let count = split.len();
        let mut owned = Vec::new();
        for (label, node) in split {
            owned.push(node.clone());
            tasks.spawn(scan_node(
                node,
                Some(label),
                pattern.to_string(),
                tuning.page_size,
                Some(tuning.node_page_timeout),
                tx.clone(),
            ));
        }
        // Keep the routing table fresh for the length of the scan.
        let watcher = client.clone();
        let every = tuning.topology_sync_every;
        tasks.spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                // A failed sync is not this task's to report: the same
                // trouble fails a node scan, which names the node, and a
                // changed table is announced on `cluster_change_rx`.
                let _ = watcher.sync_cluster().await;
            }
        });
        Nodes {
            rx,
            count,
            tasks,
            owned,
        }
    }

    /// Stop every task and close what this scan opened.
    async fn stop(mut self) {
        self.tasks.abort_all();
        close(std::mem::take(&mut self.owned)).await;
    }
}

/// One node's cursor, page by page, until it is exhausted. `label` names the
/// node in an error (`None` for the single connection of a standalone
/// server, where there is nothing to disambiguate); `timeout` bounds one
/// page.
async fn scan_node(
    node: Client,
    label: Option<String>,
    pattern: String,
    page_size: u32,
    timeout: Option<Duration>,
    events: mpsc::Sender<NodeEvent>,
) {
    let named = |what: String| match &label {
        Some(label) => format!("{label}: {what}"),
        None => what,
    };
    let mut pages = node.scan(pattern, Some(page_size), None);
    loop {
        let next = match timeout {
            Some(limit) => match tokio::time::timeout(limit, pages.next()).await {
                Ok(next) => next,
                Err(_) => {
                    let _ = events
                        .send(NodeEvent::Failed(named(format!(
                            "no answer to SCAN within {}s",
                            limit.as_secs()
                        ))))
                        .await;
                    return;
                }
            },
            None => pages.next().await,
        };
        // The stream returning `None` is this node's end; a page's
        // `has_more()` is never consulted for it.
        let Some(page) = next else {
            let _ = events.send(NodeEvent::Done).await;
            return;
        };
        let mut page = match page {
            Ok(page) => page,
            Err(e) => {
                let _ = events
                    .send(NodeEvent::Failed(named(describe_scan_error(&e))))
                    .await;
                return;
            }
        };
        if let Some(keys) = page.take_results() {
            let batch: Vec<Vec<u8>> = keys.into_iter().map(|k| k.into_bytes().to_vec()).collect();
            if events.send(NodeEvent::Keys(batch)).await.is_err() {
                return; // the page loop is gone
            }
        }
        // Ask for the next page only now, which is what keeps memory bounded by
        // the Loaded set rather than by however fast the server can talk. If
        // this is never called the scan continues on drop, which would be the
        // unbounded behaviour we are avoiding. (A no-op on a page that
        // reports no more; the stream then ends by itself.)
        page.next();
    }
}

/// Resolves when the Cluster's topology changes; never resolves on a client
/// with nothing to watch, or once the channel has closed.
async fn topology_changed(changes: &mut Option<Changes>) {
    use tokio::sync::broadcast::error::RecvError;
    let Some(rx) = changes.as_mut() else {
        return std::future::pending().await;
    };
    match rx.recv().await {
        // A lagged receiver missed changes, which is still a change.
        Ok(_) | Err(RecvError::Lagged(_)) => {}
        Err(RecvError::Closed) => std::future::pending().await,
    }
}

fn changed_now(changes: &mut Option<Changes>) -> bool {
    use tokio::sync::broadcast::error::TryRecvError;
    changes
        .as_mut()
        .is_some_and(|rx| matches!(rx.try_recv(), Ok(_) | Err(TryRecvError::Lagged(_))))
}

/// One connection per primary in the routing table, each named by its
/// address and initialised concurrently under `limit`. `Err` names the node
/// that could not be reached.
///
/// These are `split_cluster` clients rather than `with_cluster_node` calls on
/// the main client: that client has one router for every node, and a dead
/// primary stalls it for all of them, so a healthy node would time out too and
/// be named in the error.
async fn connect_nodes(client: &Client, limit: Duration) -> Result<Vec<(String, Client)>, String> {
    let split = client
        .split_cluster()
        .map_err(|e| describe_scan_error(&e))?;
    let attempts = split.into_iter().map(|node| async move {
        let label = node
            .client_config()
            .server
            .hosts()
            .first()
            .map_or_else(|| "a cluster node".to_string(), |s| s.to_string());
        let failed = match tokio::time::timeout(limit, node.init()).await {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(describe_scan_error(&e)),
            Err(_) => Some(format!("no connection within {}s", limit.as_secs())),
        };
        (label, node, failed)
    });
    let results = futures::future::join_all(attempts).await;
    let mut nodes = Vec::new();
    let mut error = None;
    for (label, node, failed) in results {
        if let (None, Some(what)) = (&error, failed) {
            error = Some(format!("{label}: {what}"));
        }
        nodes.push((label, node));
    }
    match error {
        None => Ok(nodes),
        Some(error) => {
            close(nodes.into_iter().map(|(_, c)| c).collect()).await;
            Err(error)
        }
    }
}

/// `DBSIZE` summed over every primary's own connection. `Err` names the node
/// that failed.
async fn cluster_dbsize(nodes: &[(String, Client)], limit: Duration) -> Result<u64, String> {
    let asks = nodes.iter().map(|(label, node)| async move {
        match tokio::time::timeout(limit, node.dbsize::<u64>()).await {
            Ok(Ok(size)) => Ok(size),
            Ok(Err(e)) => Err(format!("{label}: {}", describe_scan_error(&e))),
            Err(_) => Err(format!(
                "{label}: no answer to DBSIZE within {}s",
                limit.as_secs()
            )),
        }
    });
    let mut total = 0;
    for size in futures::future::join_all(asks).await {
        total += size?;
    }
    Ok(total)
}

/// Close connections this scan opened. Bounded: a dead node must not hold the
/// scan's exit open.
async fn close(nodes: Vec<Client>) {
    for node in nodes {
        let _ = tokio::time::timeout(Duration::from_secs(2), node.quit()).await;
    }
}

/// Server states that are not really errors deserve their own words (ADR-0009).
fn describe_scan_error(e: &Error) -> String {
    let details = e.details();
    if details.starts_with("LOADING") {
        "server is loading its dataset".into()
    } else if details.starts_with("BUSY") {
        "server is busy running a script".into()
    } else if details.is_empty() {
        format!("{e}")
    } else {
        details.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_and_busy_are_explained_rather_than_echoed() {
        let loading = Error::new(ErrorKind::Unknown, "LOADING Redis is loading the dataset");
        assert_eq!(
            describe_scan_error(&loading),
            "server is loading its dataset"
        );

        let busy = Error::new(ErrorKind::Unknown, "BUSY Redis is busy running a script");
        assert_eq!(
            describe_scan_error(&busy),
            "server is busy running a script"
        );
    }

    #[test]
    fn an_ordinary_error_is_passed_through() {
        let other = Error::new(ErrorKind::Unknown, "NOPERM this user has no permissions");
        assert!(describe_scan_error(&other).contains("NOPERM"));
    }
}
