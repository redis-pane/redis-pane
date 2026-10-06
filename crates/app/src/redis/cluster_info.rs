//! The Cluster Dashboard's poll (R6.3, R1.11, M5 task 7, ADR-0022,
//! `docs/plans/m5-dashboard.md`): `INFO default` on every node, concurrently,
//! plus one `CLUSTER INFO`.
//!
//! **Per-node connections, never the main client.** The main client has one
//! router for every node: a dead node stalls commands bound for healthy ones, and
//! a timeout then blames whichever node happened to be asked (task 3's finding,
//! `scan.rs`). So each node gets its own client, built from the main client's
//! config exactly as `split_cluster` builds them, and each wait carries its own
//! timeout naming the node. A dead node is one failed row; the others render.
//! The main client is only *read from* (its cached routing table, a local
//! lookup), never sent a command, so the `fred` ASK wedge
//! (`liveness::is_wedged`) cannot stall this poll.
//!
//! **The node list comes from `CLUSTER NODES`**, not `cached_cluster_state()`:
//! the routing table lists primaries only (replicas only with `fred`'s
//! `replicas` feature, and then only once replicated), while `CLUSTER NODES`
//! names every node with its role, and the role of a node that then fails its
//! `INFO` is still known. (Task 6 made the same choice for the replica rule.)
//! The primaries in the routing table only seed the first connections.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use fred::prelude::*;
use fred::types::InfoKind;
use fred::types::config::{Server, ServerConfig};
use redis_pane_core::state::{NodeReading, NodeRole};

use super::read::parse_info;
use super::scan::{close, describe_scan_error};

/// How long one node may take to connect or answer before its row fails,
/// naming it. Generous on purpose (the product is used over SSH, CLAUDE.md), and
/// well under the 2s poll's patience only because polls never stack.
pub const NODE_TIMEOUT: Duration = Duration::from_secs(4);

/// One poll's answer.
#[derive(Debug)]
pub struct ClusterPoll {
    pub nodes: Vec<NodeReading>,
    /// `CLUSTER INFO`'s text, or why it could not be read.
    pub health: Result<String, String>,
}

/// The per-node connections, kept between polls and dropped with the view.
#[derive(Default)]
pub struct NodePool {
    clients: HashMap<String, Client>,
}

impl NodePool {
    /// Close every connection, bounded: a dead node must not hold this open.
    pub async fn close(&mut self) {
        let all: Vec<Client> = self.clients.drain().map(|(_, c)| c).collect();
        close(all).await;
    }

    async fn evict(&mut self, addr: &str) {
        if let Some(c) = self.clients.remove(addr) {
            close(vec![c]).await;
        }
    }

    /// Connect to every address not already held, concurrently, each bounded by
    /// `limit`. Returns the reason for each that could not be reached.
    async fn ensure(
        &mut self,
        main: &Client,
        addrs: &[String],
        limit: Duration,
    ) -> BTreeMap<String, String> {
        let missing: Vec<&String> = addrs
            .iter()
            .filter(|a| !self.clients.contains_key(*a))
            .collect();
        let attempts = missing.into_iter().map(|addr| async move {
            let result = connect(main, addr, limit).await;
            (addr.clone(), result)
        });
        let mut failed = BTreeMap::new();
        for (addr, result) in futures::future::join_all(attempts).await {
            match result {
                Ok(client) => {
                    self.clients.insert(addr, client);
                }
                Err(why) => {
                    failed.insert(addr, why);
                }
            }
        }
        failed
    }
}

/// A connection to one node, built from the main client's config (credentials,
/// TLS, RESP3, timeouts) the way `split_cluster` builds its own, but for any
/// address and with no reconnect policy.
async fn connect(main: &Client, addr: &str, limit: Duration) -> Result<Client, String> {
    let (host, port) = addr
        .rsplit_once(':')
        .and_then(|(h, p)| Some((h, p.parse::<u16>().ok()?)))
        .ok_or_else(|| format!("{addr} is not a host:port"))?;
    let mut config = main.client_config();
    config.server = ServerConfig::Centralized {
        server: Server::new(host, port),
    };
    let client = Client::new(
        config,
        Some(main.perf_config()),
        Some(main.connection_config().clone()),
        None,
    );
    match tokio::time::timeout(limit, client.init()).await {
        Ok(Ok(_)) => Ok(client),
        Ok(Err(e)) => {
            close(vec![client]).await;
            Err(describe_scan_error(&e))
        }
        Err(_) => {
            close(vec![client]).await;
            Err(format!("no connection within {}s", limit.as_secs()))
        }
    }
}

/// One node as `CLUSTER NODES` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedNode {
    pub addr: String,
    pub role: NodeRole,
    pub slots: u32,
}

/// The shell's handle on the Cluster poll: the per-node connections, and the
/// poll in flight. One per process; the Dashboard is the only user.
#[derive(Default, Clone)]
pub struct Poller {
    pool: std::sync::Arc<tokio::sync::Mutex<NodePool>>,
    task: std::sync::Arc<std::sync::Mutex<Option<tokio::task::AbortHandle>>>,
}

impl Poller {
    /// Start a poll and send its answer in as a `Msg`. A poll still running is
    /// aborted first: its token is stale the moment a newer one is minted.
    pub fn start(
        &self,
        main: Client,
        tx: tokio::sync::mpsc::Sender<redis_pane_core::Msg>,
        clock: std::sync::Arc<dyn redis_pane_core::clock::Clock>,
        token: redis_pane_core::command::InfoToken,
    ) {
        use redis_pane_core::Msg;
        let pool = self.pool.clone();
        let handle = tokio::spawn(async move {
            let msg = {
                let mut pool = pool.lock().await;
                match poll(&main, &mut pool, NODE_TIMEOUT).await {
                    Ok(p) => Msg::ClusterInfoLoaded {
                        nodes: p.nodes,
                        health: p.health,
                        at_ms: clock.now_epoch_ms(),
                        token,
                    },
                    Err(detail) => Msg::ServerInfoFailed {
                        detail,
                        at_ms: clock.now_epoch_ms(),
                        token,
                    },
                }
            };
            let _ = tx.send(msg).await;
        });
        let previous = self
            .task
            .lock()
            .ok()
            .and_then(|mut t| t.replace(handle.abort_handle()));
        if let Some(previous) = previous {
            previous.abort();
        }
    }

    /// The Dashboard was left (or the main client replaced): abort the poll in
    /// flight and close the per-node connections, bounded, in the background.
    pub fn cancel(&self) {
        if let Some(task) = self.task.lock().ok().and_then(|mut t| t.take()) {
            task.abort();
        }
        let pool = self.pool.clone();
        tokio::spawn(async move {
            pool.lock().await.close().await;
        });
    }
}

/// Parse `CLUSTER NODES`. `fallback_host` stands in for an empty host (a node
/// that does not know its own address reports `:port`). Nodes with no usable
/// address (`noaddr`) and lines that are not a node are skipped; a hostname
/// announced after the comma is used when the IP field is empty.
pub fn parse_cluster_nodes(text: &str, fallback_host: &str) -> Vec<ListedNode> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            continue;
        }
        let flags: Vec<&str> = f[2].split(',').collect();
        if flags.contains(&"noaddr") {
            continue;
        }
        let role = if flags.contains(&"master") {
            NodeRole::Primary
        } else if flags.contains(&"slave") {
            NodeRole::Replica
        } else {
            continue;
        };
        // `ip:port@cport[,hostname]`
        let (endpoint, hostname) = match f[1].split_once(',') {
            Some((e, h)) => (e, Some(h)),
            None => (f[1], None),
        };
        let hostport = endpoint.split('@').next().unwrap_or(endpoint);
        let Some((host, port)) = hostport.rsplit_once(':') else {
            continue;
        };
        if port.parse::<u16>().is_err() || port == "0" {
            continue;
        }
        let host = if !host.is_empty() {
            host
        } else if let Some(h) = hostname.filter(|h| !h.is_empty()) {
            h
        } else {
            fallback_host
        };
        let slots = f[8..]
            .iter()
            .filter(|t| !t.starts_with('['))
            .map(|t| match t.split_once('-') {
                Some((a, b)) => match (a.parse::<u32>(), b.parse::<u32>()) {
                    (Ok(a), Ok(b)) if b >= a => b - a + 1,
                    _ => 0,
                },
                None => u32::from(t.parse::<u32>().is_ok()),
            })
            .sum();
        out.push(ListedNode {
            addr: format!("{host}:{port}"),
            role,
            slots,
        });
    }
    out
}

/// The routing table's primaries, the seeds of the first poll.
fn seeds(main: &Client) -> Vec<String> {
    let Some(routing) = main.cached_cluster_state() else {
        return Vec::new();
    };
    let mut out: Vec<String> = routing
        .slots()
        .iter()
        .map(|r| r.primary.to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Poll the Cluster once.
///
/// `Err` is the poll failing as a whole (no node could say what the Cluster
/// is), reported as the Dashboard's own error with the last good nodes kept. A
/// node failing is not an `Err`: it is a [`NodeReading`] carrying the reason.
pub async fn poll(
    main: &Client,
    pool: &mut NodePool,
    limit: Duration,
) -> Result<ClusterPoll, String> {
    // 1. Connections to every node already known and to the routing table's
    //    primaries: the ones that can say what the Cluster looks like.
    let mut seed_addrs = seeds(main);
    for known in pool.clients.keys() {
        if !seed_addrs.contains(known) {
            seed_addrs.push(known.clone());
        }
    }
    if seed_addrs.is_empty() {
        return Err("the client has no routing table for this Cluster yet".to_string());
    }
    let seed_failures = pool.ensure(main, &seed_addrs, limit).await;

    // 2. `CLUSTER NODES` and `CLUSTER INFO` from whichever seed answers first.
    let asks: Vec<_> = seed_addrs
        .iter()
        .filter_map(|a| pool.clients.get(a).map(|c| (a.clone(), c.clone())))
        .map(|(addr, client)| {
            Box::pin(async move {
                let nodes =
                    match tokio::time::timeout(limit, client.cluster_nodes::<String>()).await {
                        Ok(Ok(text)) => text,
                        Ok(Err(e)) => {
                            return Err(format!(
                                "CLUSTER NODES on {addr}: {}",
                                describe_scan_error(&e)
                            ));
                        }
                        Err(_) => {
                            return Err(format!(
                                "CLUSTER NODES on {addr}: no answer within {}s",
                                limit.as_secs()
                            ));
                        }
                    };
                let health =
                    match tokio::time::timeout(limit, client.cluster_info::<String>()).await {
                        Ok(Ok(text)) => Ok(text),
                        Ok(Err(e)) => Err(format!(
                            "CLUSTER INFO on {addr}: {}",
                            describe_scan_error(&e)
                        )),
                        Err(_) => Err(format!(
                            "CLUSTER INFO on {addr}: no answer within {}s",
                            limit.as_secs()
                        )),
                    };
                Ok((addr, nodes, health))
            })
        })
        .collect();
    if asks.is_empty() {
        let why: Vec<String> = seed_failures
            .iter()
            .map(|(a, e)| format!("{a}: {e}"))
            .collect();
        return Err(format!(
            "no node could be reached to list the Cluster ({})",
            why.join("; ")
        ));
    }
    let (answered, nodes_text, health) = match futures::future::select_ok(asks).await {
        Ok(((addr, nodes, health), _rest)) => (addr, nodes, health),
        Err(last) => {
            return Err(format!("CLUSTER NODES failed on every node: {last}"));
        }
    };

    // 3. The nodes, each with its role, and a connection to every one.
    let fallback = answered.rsplit_once(':').map_or("", |(h, _)| h);
    let listed = parse_cluster_nodes(&nodes_text, fallback);
    if listed.is_empty() {
        return Err(format!(
            "CLUSTER NODES on {answered} listed no nodes: {}",
            nodes_text.trim().chars().take(120).collect::<String>()
        ));
    }
    let addrs: Vec<String> = listed.iter().map(|n| n.addr.clone()).collect();
    let stale: Vec<String> = pool
        .clients
        .keys()
        .filter(|k| !addrs.contains(k))
        .cloned()
        .collect();
    for gone in stale {
        pool.evict(&gone).await;
    }
    let mut connect_failures = seed_failures;
    connect_failures.extend(pool.ensure(main, &addrs, limit).await);

    // 4. `INFO default` on every node, concurrently, each bounded and named.
    let asks = listed.iter().map(|node| {
        let client = pool.clients.get(&node.addr).cloned();
        let refused = connect_failures.get(&node.addr).cloned();
        async move {
            let result = match (client, refused) {
                (Some(client), _) => {
                    match tokio::time::timeout(
                        limit,
                        client.info::<String>(Some(InfoKind::Default)),
                    )
                    .await
                    {
                        Ok(Ok(text)) => Ok(parse_info(&text)),
                        Ok(Err(e)) => Err(describe_scan_error(&e)),
                        Err(_) => Err(format!("no answer within {}s", limit.as_secs())),
                    }
                }
                (None, Some(why)) => Err(why),
                (None, None) => Err("not connected".to_string()),
            };
            (node, result)
        }
    });
    let mut readings = Vec::new();
    let mut broken = Vec::new();
    for (node, result) in futures::future::join_all(asks).await {
        if result.is_err() {
            // A connection that failed is rebuilt next poll, not trusted.
            broken.push(node.addr.clone());
        }
        readings.push(NodeReading {
            addr: node.addr.clone(),
            role: node.role,
            slots: node.slots,
            info: result,
        });
    }
    for addr in broken {
        pool.evict(&addr).await;
    }
    Ok(ClusterPoll {
        nodes: readings,
        health,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODES: &str = "\
07c37dfeb235213a872192d90877d0cd55635b91 127.0.0.1:7100@17100 myself,master - 0 1426238317239 4 connected 0-5460
67ed2db8d677e59ec4a4cefb06858cf2a1a89fa1 127.0.0.1:7101@17101 master - 0 1426238316232 2 connected 5461-10922
292f8b365bb7edb5e285caf0b7e6ddc7265d2f4f 127.0.0.1:7102@17102 master - 0 1426238318243 3 connected 10923-16383
6ec23923021cf3ffec47632106199cb7f496ce01 127.0.0.1:7103@17103 slave 67ed2db8d677e59ec4a4cefb06858cf2a1a89fa1 0 1426238316232 5 connected
824fe116063bc5fcf9f4ffd895bc17aee7731ac3 127.0.0.1:7104@17104 slave 292f8b365bb7edb5e285caf0b7e6ddc7265d2f4f 0 1426238317741 6 connected
";

    #[test]
    fn every_node_comes_with_its_role_and_slot_count() {
        let nodes = parse_cluster_nodes(NODES, "ignored");
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[0].addr, "127.0.0.1:7100");
        assert_eq!(nodes[0].role, NodeRole::Primary);
        assert_eq!(nodes[0].slots, 5461);
        assert_eq!(nodes[2].slots, 5461);
        assert_eq!(nodes[3].role, NodeRole::Replica);
        assert_eq!(nodes[3].slots, 0);
    }

    #[test]
    fn single_slots_count_and_migration_markers_do_not() {
        let text = "id 10.0.0.1:7000@17000 master - 0 0 1 connected 0-9 12 [93->-aaa] [77-<-bbb]\n";
        let nodes = parse_cluster_nodes(text, "x");
        assert_eq!(nodes[0].slots, 11);
    }

    #[test]
    fn a_hostname_after_the_comma_stands_in_for_an_empty_host() {
        let text = "id :7000@17000,cache-1.example.com master - 0 0 1 connected 0-1\n";
        assert_eq!(
            parse_cluster_nodes(text, "seed")[0].addr,
            "cache-1.example.com:7000"
        );
        let text = "id :7000@17000 master - 0 0 1 connected 0-1\n";
        assert_eq!(parse_cluster_nodes(text, "seed")[0].addr, "seed:7000");
    }

    #[test]
    fn a_failed_node_is_kept_and_a_noaddr_one_is_not() {
        let text = "\
a 10.0.0.1:7000@17000 master,fail - 0 0 1 disconnected 0-100
b :0@0 slave,noaddr a 0 0 1 disconnected
";
        let nodes = parse_cluster_nodes(text, "x");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].addr, "10.0.0.1:7000");
        assert_eq!(nodes[0].role, NodeRole::Primary);
    }

    #[test]
    fn garbage_lists_nothing() {
        assert!(parse_cluster_nodes("", "x").is_empty());
        assert!(parse_cluster_nodes("ERR unknown command", "x").is_empty());
    }
}
