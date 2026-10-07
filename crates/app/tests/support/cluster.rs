//! A real Redis Cluster for the integration suite (M5 task 1, ADR-0022).
//!
//! One `redis:8.4-alpine` container runs six `redis-server` processes, three
//! primaries and three replicas. Cluster nodes announce addresses the client
//! then dials (`MOVED`, `CLUSTER SLOTS`), so those addresses must be reachable
//! from the test process. Container IPs are not routable from the host on
//! Docker Desktop, so the harness uses **identity port mapping**: it picks six
//! free host ports, runs node `i` on exactly that port inside the container,
//! maps `host:N -> container:N`, and announces `127.0.0.1`. Between themselves
//! the nodes use loopback inside the container; the test process reaches every
//! announced address as-is. The bus ports are chosen the same way but never
//! mapped (a bus port of `N + 10000` would overflow 65535 for ephemeral `N`).

use std::collections::BTreeSet;
use std::net::TcpListener;
use std::time::{Duration, Instant};

use fred::prelude::*;
use fred::types::RespVersion;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const STARTED: &str = "CLUSTER_NODES_STARTED";
const DEADLINE: Duration = Duration::from_secs(60);

/// One row of `CLUSTER NODES`.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub port: u16,
    pub is_primary: bool,
    /// For a replica, the id of the primary it follows.
    pub primary_id: Option<String>,
    /// Inclusive slot ranges owned (primaries only).
    pub slots: Vec<(u16, u16)>,
}

impl Node {
    pub fn owns(&self, slot: u16) -> bool {
        self.slots.iter().any(|&(a, b)| (a..=b).contains(&slot))
    }
}

pub struct Cluster {
    container: ContainerAsync<GenericImage>,
    /// Client ports of all six nodes, in start order.
    pub ports: Vec<u16>,
}

/// Ports for the cluster, from 20000-31999: below the 32768-60999 range Docker
/// hands out for its own random host mappings. Drawing from the OS ephemeral
/// range (bind to port 0) collides with those, and a stale forward from a
/// just-removed cluster container then shadows an unrelated test's container.
/// Every call advances past the previous one, so ports are not reused within
/// a run.
fn free_ports(n: usize) -> Vec<u16> {
    use std::sync::atomic::{AtomicU16, Ordering};
    const LO: u16 = 20_000;
    const SPAN: u16 = 12_000;
    static NEXT: AtomicU16 = AtomicU16::new(0);
    if NEXT.load(Ordering::Relaxed) == 0 {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ (std::process::id() << 8))
            .unwrap_or(1);
        let _ = NEXT.compare_exchange(
            0,
            1 + (seed % u32::from(SPAN)) as u16,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
    let mut picked = Vec::new();
    let mut held = Vec::new();
    while picked.len() < n {
        let port = LO + (NEXT.fetch_add(1, Ordering::Relaxed) % SPAN);
        // Keep each listener until all are chosen so no two picks collide.
        if let Ok(l) = TcpListener::bind(("127.0.0.1", port)) {
            held.push(l);
            picked.push(port);
        }
    }
    picked
}

/// Start a 3-primary / 3-replica cluster and wait for `cluster_state:ok` on
/// every node.
pub async fn start_cluster() -> Cluster {
    // A port is free when picked and can be taken before Docker binds it (by
    // another container's random mapping, say), so a failed start retries
    // with fresh ports rather than failing the test.
    let mut last = String::new();
    for _ in 0..5 {
        match try_start().await {
            Ok(cluster) => return form(cluster).await,
            Err(e) => last = e,
        }
    }
    panic!("docker must be running for the integration suite: {last}");
}

async fn try_start() -> Result<Cluster, String> {
    // `--daemonize yes` returns once the server has forked, not once it is
    // listening, so the script waits for every node to answer PING (30s
    // each at most) before announcing. Without that, a slow runner reached
    // `--cluster create` with the last node still binding its port.
    let all = free_ports(12);
    let (ports, bus) = all.split_at(6);
    let script = format!(
        "i=0; for p in {ports}; do b=$(echo {bus} | cut -d' ' -f$((i+1))); i=$((i+1)); \
         mkdir -p /data/$p; \
         redis-server --port $p --cluster-port $b --cluster-enabled yes \
           --cluster-config-file /data/$p/nodes.conf --dir /data/$p \
           --cluster-announce-ip 127.0.0.1 --cluster-node-timeout 3000 \
           --bind 0.0.0.0 --protected-mode no --appendonly no --save '' \
           --daemonize yes --logfile /data/$p/log; \
         done; \
         for p in {ports}; do n=0; \
           until redis-cli -p $p ping >/dev/null 2>&1; do \
             n=$((n+1)); [ $n -gt 300 ] && exit 1; sleep 0.1; \
           done; \
         done; echo {STARTED}; exec tail -f /dev/null",
        ports = join(ports),
        bus = join(bus),
    );
    let mut image = GenericImage::new("redis", "8.4-alpine")
        .with_wait_for(WaitFor::message_on_stdout(STARTED))
        .with_entrypoint("sh")
        .with_cmd(vec!["-c".to_string(), script]);
    for &p in ports {
        image = image.with_mapped_port(p, ContainerPort::Tcp(p));
    }
    let container = image.start().await.map_err(|e| e.to_string())?;
    Ok(Cluster {
        container,
        ports: ports.to_vec(),
    })
}

async fn form(cluster: Cluster) -> Cluster {
    let ports = cluster.ports.clone();
    let mut args: Vec<String> = vec!["redis-cli".into(), "--cluster".into(), "create".into()];
    args.extend(ports.iter().map(|p| format!("127.0.0.1:{p}")));
    args.extend(
        ["--cluster-replicas", "1", "--cluster-yes"]
            .iter()
            .map(|s| s.to_string()),
    );
    let out = cluster.exec(args).await;
    assert!(
        out.contains("[OK] All 16384 slots covered"),
        "cluster create: {out}"
    );
    cluster.wait_ready().await;
    cluster
}

fn join(ports: &[u16]) -> String {
    ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

impl Cluster {
    /// The seed URL for a plain cluster client.
    pub fn seed_url(&self) -> String {
        format!("redis-cluster://127.0.0.1:{}", self.ports[0])
    }

    /// Run a command inside the container; panics on a non-zero exit.
    pub async fn exec(&self, args: Vec<String>) -> String {
        let shown = args.join(" ");
        let mut res = self
            .container
            .exec(testcontainers::core::ExecCommand::new(args))
            .await
            .unwrap_or_else(|e| panic!("exec `{shown}`: {e}"));
        let out = String::from_utf8_lossy(&res.stdout_to_vec().await.unwrap()).to_string();
        let err = String::from_utf8_lossy(&res.stderr_to_vec().await.unwrap()).to_string();
        let code = res.exit_code().await.unwrap();
        assert_eq!(code, Some(0), "`{shown}` failed: {out}{err}");
        out
    }

    /// `redis-cli -p <port> <args...>` inside the container.
    pub async fn cli(&self, port: u16, args: &[&str]) -> String {
        let mut v = vec!["redis-cli".to_string(), "-p".into(), port.to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        self.exec(v).await
    }

    /// Poll until every node reports `cluster_state:ok` and sees the 3+3 shape, with a deadline (never a sleep for a fixed time).
    pub async fn wait_ready(&self) {
        let start = Instant::now();
        loop {
            let mut ok = true;
            for &p in &self.ports {
                let info = self.cli(p, &["cluster", "info"]).await;
                if !(info.contains("cluster_state:ok") && info.contains("cluster_known_nodes:6")) {
                    ok = false;
                    break;
                }
            }
            if ok {
                // `create` returns before the replicas finish attaching, so
                // "ok" alone can still show six primaries. Every node must see
                // 3 slot-owning primaries and 3 attached replicas.
                let mut shaped = true;
                for &p in &self.ports {
                    let nodes = self.nodes_from(p).await;
                    let primaries = nodes
                        .iter()
                        .filter(|n| n.is_primary && !n.slots.is_empty())
                        .count();
                    let replicas = nodes.iter().filter(|n| n.primary_id.is_some()).count();
                    if nodes.len() != 6 || primaries != 3 || replicas != 3 {
                        shaped = false;
                        break;
                    }
                }
                if shaped {
                    return;
                }
            }
            assert!(
                start.elapsed() < DEADLINE,
                "cluster did not reach cluster_state:ok within {DEADLINE:?}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// The current topology, as seen from the first node.
    pub async fn nodes(&self) -> Vec<Node> {
        self.nodes_from(self.ports[0]).await
    }

    pub async fn nodes_from(&self, port: u16) -> Vec<Node> {
        let raw = self.cli(port, &["cluster", "nodes"]).await;
        raw.lines().filter_map(parse_node).collect()
    }

    pub async fn primaries(&self) -> Vec<Node> {
        self.nodes()
            .await
            .into_iter()
            .filter(|n| n.is_primary)
            .collect()
    }

    pub async fn replicas(&self) -> Vec<Node> {
        self.nodes()
            .await
            .into_iter()
            .filter(|n| !n.is_primary)
            .collect()
    }

    /// The node (by port) that owns `slot` right now.
    pub async fn owner_of_slot(&self, slot: u16) -> Node {
        self.primaries()
            .await
            .into_iter()
            .find(|n| n.owns(slot))
            .expect("every slot has an owner")
    }

    /// A key name whose slot is currently owned by `primary`.
    pub async fn key_in_slot_of(&self, primary: u16) -> String {
        let node = self
            .primaries()
            .await
            .into_iter()
            .find(|n| n.port == primary)
            .expect("not a primary");
        (0u32..)
            .map(|i| format!("rp:key:{i}"))
            .find(|k| node.owns(fred::util::redis_keyslot(k.as_bytes())))
            .unwrap()
    }

    /// A plain `fred` cluster client (RESP3) built from the seed URL.
    pub async fn client(&self) -> Client {
        let mut config = Config::from_url(&self.seed_url()).expect("cluster url");
        config.version = RespVersion::RESP3;
        let client = Builder::from_config(config).build().expect("client");
        client.init().await.expect("cluster client init");
        client
    }

    /// Promote `replica` with `CLUSTER FAILOVER` and wait until it is a primary
    /// and every node agrees the cluster is ok again.
    ///
    /// Under heavy host load a plain `CLUSTER FAILOVER` (which waits for the
    /// replica to catch up and for a manual-failover handshake with the primary)
    /// has been seen not to complete in 60s. So the command is re-sent if the
    /// replica is still not promoted after `ATTEMPT`, and from the second
    /// attempt on as `FORCE` (no primary handshake). The overall deadline is
    /// longer. A replica that never promotes still fails the test, naming the
    /// attempts made.
    pub async fn failover(&self, replica: u16) {
        const ATTEMPT: Duration = Duration::from_secs(20);
        const OVERALL: Duration = Duration::from_secs(120);
        let start = Instant::now();
        let mut attempts = 0u32;
        let mut last_sent: Option<Instant> = None;
        loop {
            let me = self
                .nodes_from(replica)
                .await
                .into_iter()
                .find(|n| n.port == replica)
                .expect("self row");
            if me.is_primary {
                break;
            }
            if last_sent.is_none_or(|t| t.elapsed() >= ATTEMPT) {
                attempts += 1;
                let args: &[&str] = if attempts == 1 {
                    &["cluster", "failover"]
                } else {
                    &["cluster", "failover", "force"]
                };
                self.cli(replica, args).await;
                last_sent = Some(Instant::now());
            }
            assert!(
                start.elapsed() < OVERALL,
                "replica {replica} was not promoted within {OVERALL:?} ({attempts} CLUSTER FAILOVER attempts)"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // The old primary demotes and every node converges on the new view.
        loop {
            self.wait_ready().await;
            if self.agree_on_slots().await {
                return;
            }
            assert!(start.elapsed() < OVERALL, "nodes never agreed on slots");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Move one slot, with its keys, from primary `from` to primary `to`
    /// (ports). Standard `IMPORTING` / `MIGRATING` / `MIGRATE` / `SETSLOT NODE`.
    pub async fn migrate_slot(&self, slot: u16, from: u16, to: u16) {
        self.begin_migration(slot, from, to).await;
        self.finish_migration(slot, to).await;
    }

    /// The first half of [`Cluster::migrate_slot`]: `IMPORTING` / `MIGRATING`
    /// and every key moved, but ownership not yet handed over. The slot is
    /// *mid-migration*: a key that has moved answers `ASK` from `from`.
    pub async fn begin_migration(&self, slot: u16, from: u16, to: u16) {
        let nodes = self.nodes().await;
        let id_of = |port: u16| {
            nodes
                .iter()
                .find(|n| n.port == port)
                .unwrap_or_else(|| panic!("no node on {port}"))
                .id
                .clone()
        };
        let (from_id, to_id) = (id_of(from), id_of(to));
        let s = slot.to_string();
        self.cli(to, &["cluster", "setslot", &s, "importing", &from_id])
            .await;
        self.cli(from, &["cluster", "setslot", &s, "migrating", &to_id])
            .await;
        loop {
            let keys = self
                .cli(from, &["cluster", "getkeysinslot", &s, "100"])
                .await;
            let keys: Vec<&str> = keys.lines().filter(|l| !l.is_empty()).collect();
            if keys.is_empty() {
                break;
            }
            let to_port = to.to_string();
            let mut args = vec!["migrate", "127.0.0.1", &to_port, "", "0", "5000", "keys"];
            args.extend(keys.iter().copied());
            self.cli(from, &args).await;
        }
    }

    /// The second half of [`Cluster::migrate_slot`]: hand `slot` to `to` and
    /// wait until every node agrees.
    pub async fn finish_migration(&self, slot: u16, to: u16) {
        let nodes = self.nodes().await;
        let to_id = nodes
            .iter()
            .find(|n| n.port == to)
            .unwrap_or_else(|| panic!("no node on {to}"))
            .id
            .clone();
        let s = slot.to_string();
        // Tell the importer first, then everyone else, so ownership converges.
        self.cli(to, &["cluster", "setslot", &s, "node", &to_id])
            .await;
        for p in self.primaries().await.iter().map(|n| n.port) {
            if p != to {
                self.cli(p, &["cluster", "setslot", &s, "node", &to_id])
                    .await;
            }
        }
        let start = Instant::now();
        loop {
            if self.agree_on_slots().await
                && self.owner_of_slot(slot).await.port == to
                && self.slot_owner_everywhere(slot, to).await
            {
                return;
            }
            assert!(
                start.elapsed() < DEADLINE,
                "slot {slot} did not settle on {to} within {DEADLINE:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn slot_owner_everywhere(&self, slot: u16, owner: u16) -> bool {
        for &p in &self.ports {
            let owned = self
                .nodes_from(p)
                .await
                .into_iter()
                .any(|n| n.port == owner && n.owns(slot));
            if !owned {
                return false;
            }
        }
        true
    }

    /// Every node reports the same primary set and slot ownership.
    async fn agree_on_slots(&self) -> bool {
        let mut views = BTreeSet::new();
        for &p in &self.ports {
            let mut v: Vec<(u16, Vec<(u16, u16)>)> = self
                .nodes_from(p)
                .await
                .into_iter()
                .filter(|n| n.is_primary)
                .map(|n| (n.port, n.slots))
                .collect();
            v.sort();
            views.insert(format!("{v:?}"));
        }
        views.len() == 1
    }
}

/// `<id> <ip:port@bus[,hostname]> <flags> <master|-> <ping> <pong> <epoch>
/// <link> [slot|a-b|[migrating..]...]`
fn parse_node(line: &str) -> Option<Node> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 8 {
        return None;
    }
    let port = f[1].split(':').nth(1)?.split('@').next()?.parse().ok()?;
    let is_primary = f[2].split(',').any(|x| x == "master");
    let slots = f[8..]
        .iter()
        .filter(|s| !s.starts_with('['))
        .filter_map(|s| match s.split_once('-') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => s.parse().ok().map(|n| (n, n)),
        })
        .collect();
    Some(Node {
        id: f[0].to_string(),
        port,
        is_primary,
        primary_id: (f[3] != "-").then(|| f[3].to_string()),
        slots,
    })
}
