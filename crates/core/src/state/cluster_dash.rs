//! The Cluster Dashboard's state and its pure aggregation (R6.3, R1.11,
//! M5 task 7, `docs/plans/m5-dashboard.md`).
//!
//! A Cluster has no single set of figures, so the Dashboard keeps one
//! [`DashboardState`] per node (its parsed `INFO`, previous poll and ops/sec
//! history, exactly as on a standalone server) and derives the Cluster's tiles
//! from them at render time. Nothing here is stored twice: a tile is a function
//! of the per-node readings, so the node view and the aggregate can never
//! disagree.
//!
//! The aggregation rules are decision 2 of the plan: memory summed over
//! primaries against the summed `maxmemory`; ops/sec summed over primaries;
//! clients summed over every node; the worst replication lag; evictions summed;
//! the hit ratio from summed hits and misses, never by averaging ratios.

use std::collections::VecDeque;

use super::dashboard::{
    AlarmLevel, ClientsTile, DashboardState, EvictionTile, HIT_RATIO_MIN_SAMPLES,
    HIT_RATIO_WARN_RATIO, HitRatioTile, MEMORY_DANGER_RATIO, MEMORY_WARN_RATIO, OPS_HISTORY_CAP,
    RawInfo,
};

/// The number of hash slots in a Redis Cluster.
pub const TOTAL_SLOTS: u32 = 16_384;

/// A node's role, from `CLUSTER NODES` (not from its own `INFO`, which a failed
/// node cannot give: the role of a node that did not answer is still known).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeRole {
    Primary,
    Replica,
}

impl NodeRole {
    pub fn label(&self) -> &'static str {
        match self {
            NodeRole::Primary => "primary",
            NodeRole::Replica => "replica",
        }
    }
}

/// What the shell learned about one node in one poll. `info` is the node's own
/// `INFO default`, or the reason it could not be read (the failing node is
/// named by `addr`; the command is always `INFO`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeReading {
    pub addr: String,
    pub role: NodeRole,
    /// Slots the node owns (primaries; 0 for a replica).
    pub slots: u32,
    pub info: Result<RawInfo, String>,
}

/// `CLUSTER INFO`, reduced to what the health line says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterHealth {
    /// `ok` or `fail`.
    pub state: String,
    pub slots_assigned: u32,
    pub slots_pfail: u32,
    pub slots_fail: u32,
}

impl ClusterHealth {
    /// Parse the text of `CLUSTER INFO`. `None` when it carries no
    /// `cluster_state`, so a reply that is not `CLUSTER INFO` cannot read as
    /// healthy.
    pub fn parse(text: &str) -> Option<Self> {
        let get = |key: &str| -> Option<&str> {
            text.lines().find_map(|l| {
                l.trim_end_matches('\r')
                    .strip_prefix(key)?
                    .strip_prefix(':')
            })
        };
        let num = |key: &str| get(key).and_then(|v| v.trim().parse::<u32>().ok());
        Some(ClusterHealth {
            state: get("cluster_state")?.trim().to_string(),
            slots_assigned: num("cluster_slots_assigned").unwrap_or(0),
            slots_pfail: num("cluster_slots_pfail").unwrap_or(0),
            slots_fail: num("cluster_slots_fail").unwrap_or(0),
        })
    }

    /// Slots the cluster itself counts as failing (`pfail` plus `fail`).
    pub fn failing(&self) -> u32 {
        self.slots_pfail + self.slots_fail
    }

    pub fn level(&self) -> AlarmLevel {
        if self.state != "ok" || self.slots_assigned < TOTAL_SLOTS || self.failing() > 0 {
            AlarmLevel::Danger
        } else {
            AlarmLevel::Ok
        }
    }

    /// `cluster_state ok · 16384/16384 slots · 0 failing`, with `sep` between
    /// the parts (the glyph set's separator).
    pub fn line(&self, sep: &str) -> String {
        format!(
            "cluster_state {} {sep} {}/{} slots {sep} {} failing",
            self.state,
            self.slots_assigned,
            TOTAL_SLOTS,
            self.failing()
        )
    }
}

/// One node's row: who it is, and its own single-node Dashboard state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterNode {
    pub addr: String,
    pub role: NodeRole,
    pub slots: u32,
    /// The node's own readings. `dash.error` is set when its last `INFO`
    /// failed, and is the row's failed marker; the last good reading stays.
    pub dash: DashboardState,
}

impl ClusterNode {
    pub fn failed(&self) -> Option<&str> {
        self.dash.error.as_deref()
    }

    /// Whether this node's figures are fresh enough to aggregate: it answered
    /// its last `INFO` and has ever had a reading.
    fn answering(&self) -> bool {
        self.dash.error.is_none() && self.dash.raw().is_some()
    }
}

/// The aggregate Memory tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterMemory {
    pub used_bytes: u64,
    pub peak_bytes: u64,
    /// The summed `maxmemory`; `None` when any answering primary has none.
    pub max_bytes: Option<u64>,
    /// How many answering primaries have no `maxmemory`.
    pub no_limit_nodes: usize,
    pub level: AlarmLevel,
}

/// The aggregate Replication tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterReplication {
    pub replicas: usize,
    pub worst_lag_secs: Option<i64>,
    pub links_down: usize,
    pub level: AlarmLevel,
}

/// Every Cluster tile, derived from the per-node readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterTiles {
    pub memory: ClusterMemory,
    pub hit_ratio: HitRatioTile,
    pub ops_per_sec: u64,
    pub clients: ClientsTile,
    pub replication: ClusterReplication,
    pub eviction: EvictionTile,
    /// Nodes whose last `INFO` failed or that never answered; excluded from
    /// every figure above.
    pub unreachable: usize,
    pub nodes: usize,
}

/// The Cluster Dashboard: every node, the health line, and where the reader is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClusterDash {
    nodes: Vec<ClusterNode>,
    health: Option<Result<ClusterHealth, String>>,
    selected: Option<String>,
    drilled: Option<String>,
    ops_history: VecDeque<u64>,
}

impl ClusterDash {
    pub fn nodes(&self) -> &[ClusterNode] {
        &self.nodes
    }

    pub fn health(&self) -> Option<&Result<ClusterHealth, String>> {
        self.health.as_ref()
    }

    pub fn ops_history(&self) -> &VecDeque<u64> {
        &self.ops_history
    }

    /// Index of the node the cursor is on (the first when none, or when the
    /// selected node left the Cluster).
    pub fn selected_index(&self) -> usize {
        self.selected
            .as_ref()
            .and_then(|a| self.nodes.iter().position(|n| &n.addr == a))
            .unwrap_or(0)
    }

    pub fn drilled_index(&self) -> Option<usize> {
        let addr = self.drilled.as_ref()?;
        self.nodes.iter().position(|n| &n.addr == addr)
    }

    pub(super) fn node_dash_mut(&mut self, i: usize) -> &mut DashboardState {
        &mut self.nodes[i].dash
    }

    pub fn drilled_node(&self) -> Option<&ClusterNode> {
        self.drilled_index().map(|i| &self.nodes[i])
    }

    pub fn cursor_down(&mut self) {
        let i = self.selected_index();
        if i + 1 < self.nodes.len() {
            self.selected = Some(self.nodes[i + 1].addr.clone());
        }
    }

    pub fn cursor_up(&mut self) {
        let i = self.selected_index();
        if i > 0 {
            self.selected = Some(self.nodes[i - 1].addr.clone());
        }
    }

    /// `Enter` on a row: show that node's own tiles.
    pub fn drill(&mut self) {
        if let Some(node) = self.nodes.get(self.selected_index()) {
            self.drilled = Some(node.addr.clone());
        }
    }

    pub fn undrill(&mut self) {
        self.drilled = None;
    }

    /// Fold one poll in. Nodes are keyed by address, so a node keeps its
    /// previous reading (the rising-counter alarms compare against it) and its
    /// ops history across polls; a node that left the Cluster is dropped.
    /// Returns the nodes that *newly* failed, for the notification (one toast
    /// per failure, not one per poll).
    pub fn record(
        &mut self,
        readings: Vec<NodeReading>,
        health: Result<String, String>,
        at_ms: u64,
    ) -> Vec<(String, String)> {
        let mut old = std::mem::take(&mut self.nodes);
        let mut newly_failed = Vec::new();
        for r in readings {
            let mut node = match old.iter().position(|n| n.addr == r.addr) {
                Some(i) => old.swap_remove(i),
                None => ClusterNode {
                    addr: r.addr.clone(),
                    role: r.role,
                    slots: r.slots,
                    dash: DashboardState::default(),
                },
            };
            node.role = r.role;
            node.slots = r.slots;
            match r.info {
                Ok(info) => node.dash.record_poll(info, at_ms),
                Err(detail) => {
                    if node.dash.error.is_none() {
                        newly_failed.push((r.addr.clone(), detail.clone()));
                    }
                    node.dash.loading = false;
                    node.dash.error = Some(detail);
                }
            }
            self.nodes.push(node);
        }
        // Primaries first, then replicas; each by address, so the table never
        // reshuffles between polls.
        self.nodes
            .sort_by(|a, b| (a.role, &a.addr).cmp(&(b.role, &b.addr)));
        self.health = Some(match health {
            Ok(text) => ClusterHealth::parse(&text)
                .ok_or_else(|| "CLUSTER INFO carried no cluster_state".to_string()),
            Err(e) => Err(e),
        });
        let ops = self.tiles().ops_per_sec;
        self.ops_history.push_back(ops);
        while self.ops_history.len() > OPS_HISTORY_CAP {
            self.ops_history.pop_front();
        }
        // The cursor and the drill-down follow a node by address; one that left
        // the Cluster releases them.
        if self
            .drilled
            .as_ref()
            .is_some_and(|a| !self.nodes.iter().any(|n| &n.addr == a))
        {
            self.drilled = None;
        }
        newly_failed
    }

    /// The Cluster's tiles, from the nodes that answered their last `INFO`.
    pub fn tiles(&self) -> ClusterTiles {
        let live: Vec<&ClusterNode> = self.nodes.iter().filter(|n| n.answering()).collect();
        let primaries = || live.iter().filter(|n| n.role == NodeRole::Primary);

        // Memory: primaries only (a replica holds the same data again).
        let (mut used, mut peak, mut max) = (0u64, 0u64, 0u64);
        let mut no_limit = 0usize;
        let mut worst_node = AlarmLevel::Ok;
        for n in primaries() {
            if let Some(m) = n.dash.memory_tile() {
                used += m.used_bytes;
                peak += m.peak_bytes;
                match m.max_bytes {
                    Some(limit) => max += limit,
                    None => no_limit += 1,
                }
                worst_node = worst_node.max(m.level);
            }
        }
        let max_bytes = (no_limit == 0 && max > 0).then_some(max);
        // The sum can hide one full node among empty ones, so the tile is at
        // least as alarming as its worst primary.
        let summed = match max_bytes {
            Some(limit) => {
                let ratio = used as f64 / limit as f64;
                if ratio >= MEMORY_DANGER_RATIO {
                    AlarmLevel::Danger
                } else if ratio >= MEMORY_WARN_RATIO {
                    AlarmLevel::Warn
                } else {
                    AlarmLevel::Ok
                }
            }
            None => AlarmLevel::Ok,
        };
        let memory = ClusterMemory {
            used_bytes: used,
            peak_bytes: peak,
            max_bytes,
            no_limit_nodes: no_limit,
            level: summed.max(worst_node),
        };

        let ops_per_sec = primaries()
            .map(|n| {
                n.dash
                    .raw()
                    .and_then(|r| r.field("instantaneous_ops_per_sec"))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0)
            })
            .sum();

        // Hit ratio, evictions, clients and replication read every node.
        let (mut hits, mut misses) = (0u64, 0u64);
        let (mut evicted, mut expired) = (0u64, 0u64);
        let (mut connected, mut blocked) = (0u64, 0u64);
        let mut clients_level = AlarmLevel::Ok;
        let mut eviction_level = AlarmLevel::Ok;
        let mut repl_level = AlarmLevel::Ok;
        let mut worst_lag: Option<i64> = None;
        let mut links_down = 0usize;
        for n in &live {
            if let Some(h) = n.dash.hit_ratio_tile() {
                hits += h.hits;
                misses += h.misses;
            }
            if let Some(e) = n.dash.eviction_tile() {
                evicted += e.evicted_keys;
                expired += e.expired_keys;
                eviction_level = eviction_level.max(e.level);
            }
            if let Some(c) = n.dash.clients_tile() {
                connected += c.connected;
                blocked += c.blocked;
                clients_level = clients_level.max(c.level);
            }
            if let Some(r) = n.dash.replication_tile() {
                repl_level = repl_level.max(r.level);
                if let Some(lag) = r.lag_secs {
                    worst_lag = Some(worst_lag.map_or(lag, |w| w.max(lag)));
                }
                if r.link_status.as_deref() == Some("down") {
                    links_down += 1;
                }
            }
        }
        let total = hits + misses;
        let hit_level = if total >= HIT_RATIO_MIN_SAMPLES
            && (hits as f64 / total as f64) < HIT_RATIO_WARN_RATIO
        {
            AlarmLevel::Warn
        } else {
            AlarmLevel::Ok
        };
        ClusterTiles {
            memory,
            hit_ratio: HitRatioTile {
                hits,
                misses,
                level: hit_level,
            },
            ops_per_sec,
            clients: ClientsTile {
                connected,
                blocked,
                level: clients_level,
            },
            replication: ClusterReplication {
                replicas: self
                    .nodes
                    .iter()
                    .filter(|n| n.role == NodeRole::Replica)
                    .count(),
                worst_lag_secs: worst_lag,
                links_down,
                level: repl_level,
            },
            eviction: EvictionTile {
                evicted_keys: evicted,
                expired_keys: expired,
                level: eviction_level,
            },
            unreachable: self.nodes.len() - live.len(),
            nodes: self.nodes.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pairs: &[(&str, &str)]) -> RawInfo {
        let field = |k: &str, v: &str| (k.to_string(), v.to_string());
        let pick = |keys: &[&str]| -> Vec<(String, String)> {
            pairs
                .iter()
                .filter(|(k, _)| keys.contains(k))
                .map(|(k, v)| field(k, v))
                .collect()
        };
        RawInfo::new(vec![
            (
                "Clients".into(),
                pick(&["connected_clients", "blocked_clients"]),
            ),
            (
                "Memory".into(),
                pick(&["used_memory", "used_memory_peak", "maxmemory"]),
            ),
            (
                "Stats".into(),
                pick(&[
                    "instantaneous_ops_per_sec",
                    "keyspace_hits",
                    "keyspace_misses",
                    "evicted_keys",
                    "expired_keys",
                    "rejected_connections",
                ]),
            ),
            (
                "Replication".into(),
                pick(&["role", "master_last_io_seconds_ago", "master_link_status"]),
            ),
        ])
    }

    fn primary(addr: &str, extra: &[(&str, &str)]) -> NodeReading {
        let mut pairs = vec![
            ("role", "master"),
            ("connected_clients", "10"),
            ("used_memory", "100"),
            ("used_memory_peak", "150"),
            ("maxmemory", "1000"),
            ("instantaneous_ops_per_sec", "5"),
            ("keyspace_hits", "0"),
            ("keyspace_misses", "0"),
            ("evicted_keys", "0"),
            ("expired_keys", "0"),
        ];
        for (k, v) in extra {
            pairs.retain(|(pk, _)| pk != k);
            pairs.push((k, v));
        }
        NodeReading {
            addr: addr.into(),
            role: NodeRole::Primary,
            slots: 5461,
            info: Ok(info(&pairs)),
        }
    }

    fn replica(addr: &str, lag: &str, link: &str) -> NodeReading {
        NodeReading {
            addr: addr.into(),
            role: NodeRole::Replica,
            slots: 0,
            info: Ok(info(&[
                ("role", "slave"),
                ("connected_clients", "3"),
                ("used_memory", "100"),
                ("maxmemory", "1000"),
                ("instantaneous_ops_per_sec", "7"),
                ("master_last_io_seconds_ago", lag),
                ("master_link_status", link),
            ])),
        }
    }

    fn failed(addr: &str, role: NodeRole) -> NodeReading {
        NodeReading {
            addr: addr.into(),
            role,
            slots: 0,
            info: Err("no answer to INFO within 4s".into()),
        }
    }

    fn ok_health() -> Result<String, String> {
        Ok("cluster_state:ok\r\ncluster_slots_assigned:16384\r\ncluster_slots_ok:16384\r\ncluster_slots_pfail:0\r\ncluster_slots_fail:0\r\n".into())
    }

    fn dash(readings: Vec<NodeReading>) -> ClusterDash {
        let mut d = ClusterDash::default();
        d.record(readings, ok_health(), 1_000);
        d
    }

    #[test]
    fn memory_sums_primaries_only_against_the_summed_maxmemory() {
        let d = dash(vec![
            primary("a:1", &[("used_memory", "100")]),
            primary("b:2", &[("used_memory", "300")]),
            replica("c:3", "0", "up"),
        ]);
        let m = d.tiles().memory;
        assert_eq!(m.used_bytes, 400, "a replica's copy is not added");
        assert_eq!(m.max_bytes, Some(2000));
        assert_eq!(m.peak_bytes, 300);
        assert_eq!(m.no_limit_nodes, 0);
    }

    #[test]
    fn a_primary_without_maxmemory_makes_the_tile_unlimited_and_counts_them() {
        let d = dash(vec![
            primary("a:1", &[]),
            primary("b:2", &[("maxmemory", "0")]),
            primary("c:3", &[("maxmemory", "0")]),
        ]);
        let m = d.tiles().memory;
        assert_eq!(m.max_bytes, None);
        assert_eq!(m.no_limit_nodes, 2);
    }

    #[test]
    fn one_full_primary_alarms_the_tile_though_the_sum_is_low() {
        let d = dash(vec![
            primary("a:1", &[("used_memory", "990")]),
            primary("b:2", &[("used_memory", "0")]),
            primary("c:3", &[("used_memory", "0")]),
        ]);
        let m = d.tiles().memory;
        assert_eq!(m.used_bytes, 990);
        assert_eq!(m.level, AlarmLevel::Danger);
    }

    #[test]
    fn the_summed_ratio_alarms_at_the_tile_thresholds() {
        let d = dash(vec![
            primary("a:1", &[("used_memory", "850")]),
            primary("b:2", &[("used_memory", "800")]),
        ]);
        // 1650 / 2000 = 82.5%: warn on the sum, and warn on node a (85%).
        assert_eq!(d.tiles().memory.level, AlarmLevel::Warn);
    }

    #[test]
    fn ops_per_sec_sum_over_primaries_not_replicas() {
        let d = dash(vec![
            primary("a:1", &[("instantaneous_ops_per_sec", "10")]),
            primary("b:2", &[("instantaneous_ops_per_sec", "32")]),
            replica("c:3", "0", "up"),
        ]);
        assert_eq!(d.tiles().ops_per_sec, 42);
    }

    #[test]
    fn clients_sum_over_every_node() {
        let d = dash(vec![
            primary("a:1", &[("connected_clients", "10")]),
            primary(
                "b:2",
                &[("connected_clients", "20"), ("blocked_clients", "2")],
            ),
            replica("c:3", "0", "up"),
        ]);
        let c = d.tiles().clients;
        assert_eq!(c.connected, 33);
        assert_eq!(c.blocked, 2);
        assert_eq!(c.level, AlarmLevel::Warn);
    }

    #[test]
    fn the_hit_ratio_comes_from_summed_counts_not_averaged_ratios() {
        // Node a: 10 of 10 = 100%. Node b: 90 of 1090 = ~8%. The average of the
        // ratios is ~54%; the true ratio is 100 / 1100 = 9.09%.
        let d = dash(vec![
            primary("a:1", &[("keyspace_hits", "10"), ("keyspace_misses", "0")]),
            primary(
                "b:2",
                &[("keyspace_hits", "90"), ("keyspace_misses", "1000")],
            ),
        ]);
        let h = d.tiles().hit_ratio;
        assert_eq!((h.hits, h.misses), (100, 1000));
        assert!((h.ratio().unwrap() - 100.0 / 1100.0).abs() < 1e-9);
        assert_eq!(h.level, AlarmLevel::Warn, "1100 samples, under 80%");
    }

    #[test]
    fn evictions_and_expiries_sum() {
        let d = dash(vec![
            primary("a:1", &[("evicted_keys", "3"), ("expired_keys", "10")]),
            primary("b:2", &[("evicted_keys", "4"), ("expired_keys", "5")]),
        ]);
        let e = d.tiles().eviction;
        assert_eq!((e.evicted_keys, e.expired_keys), (7, 15));
    }

    #[test]
    fn evictions_rising_on_any_node_warn() {
        let mut d = dash(vec![
            primary("a:1", &[("evicted_keys", "3")]),
            primary("b:2", &[("evicted_keys", "0")]),
        ]);
        d.record(
            vec![
                primary("a:1", &[("evicted_keys", "3")]),
                primary("b:2", &[("evicted_keys", "1")]),
            ],
            ok_health(),
            3_000,
        );
        assert_eq!(d.tiles().eviction.level, AlarmLevel::Warn);
    }

    #[test]
    fn replication_reports_the_worst_lag_and_down_links() {
        let d = dash(vec![
            primary("a:1", &[]),
            replica("b:2", "3", "up"),
            replica("c:3", "12", "up"),
            replica("d:4", "0", "down"),
        ]);
        let r = d.tiles().replication;
        assert_eq!(r.replicas, 3);
        assert_eq!(r.worst_lag_secs, Some(12));
        assert_eq!(r.links_down, 1);
        assert_eq!(r.level, AlarmLevel::Danger, "a down link is danger");
    }

    #[test]
    fn a_failed_node_keeps_its_row_and_is_left_out_of_every_figure() {
        let d = dash(vec![
            primary("a:1", &[("used_memory", "100")]),
            primary("b:2", &[("used_memory", "100")]),
        ]);
        let mut d = d;
        d.record(
            vec![
                primary("a:1", &[("used_memory", "100")]),
                failed("b:2", NodeRole::Primary),
            ],
            ok_health(),
            3_000,
        );
        assert_eq!(d.nodes().len(), 2);
        assert!(d.nodes()[1].failed().is_some());
        let t = d.tiles();
        assert_eq!(t.memory.used_bytes, 100, "the stale reading is not summed");
        assert_eq!(t.unreachable, 1);
    }

    #[test]
    fn only_a_new_failure_is_reported_for_a_notification() {
        let mut d = dash(vec![primary("a:1", &[])]);
        let first = d.record(vec![failed("a:1", NodeRole::Primary)], ok_health(), 2);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, "a:1");
        let again = d.record(vec![failed("a:1", NodeRole::Primary)], ok_health(), 3);
        assert!(again.is_empty(), "no toast per poll");
        let healed = d.record(vec![primary("a:1", &[])], ok_health(), 4);
        assert!(healed.is_empty());
        assert!(d.nodes()[0].failed().is_none());
    }

    #[test]
    fn a_node_that_never_answered_has_no_figures() {
        let d = dash(vec![failed("a:1", NodeRole::Primary)]);
        let t = d.tiles();
        assert_eq!(t.memory.used_bytes, 0);
        assert_eq!(t.unreachable, 1);
    }

    #[test]
    fn rows_order_primaries_first_then_replicas_by_address() {
        let d = dash(vec![
            replica("a:9", "0", "up"),
            primary("z:1", &[]),
            primary("b:1", &[]),
        ]);
        let order: Vec<_> = d.nodes().iter().map(|n| n.addr.as_str()).collect();
        assert_eq!(order, ["b:1", "z:1", "a:9"]);
    }

    #[test]
    fn the_cursor_follows_its_node_across_polls_and_drill_down_works() {
        let mut d = dash(vec![primary("a:1", &[]), primary("b:2", &[])]);
        d.cursor_down();
        assert_eq!(d.selected_index(), 1);
        d.cursor_down();
        assert_eq!(d.selected_index(), 1, "stops at the last row");
        d.drill();
        assert_eq!(d.drilled_node().unwrap().addr, "b:2");
        d.record(
            vec![primary("b:2", &[]), primary("a:1", &[])],
            ok_health(),
            5,
        );
        assert_eq!(d.drilled_node().unwrap().addr, "b:2");
        d.undrill();
        assert!(d.drilled_node().is_none());
        d.cursor_up();
        d.cursor_up();
        assert_eq!(d.selected_index(), 0);
    }

    #[test]
    fn a_drilled_node_that_left_the_cluster_releases_the_drill_down() {
        let mut d = dash(vec![primary("a:1", &[]), primary("b:2", &[])]);
        d.cursor_down();
        d.drill();
        d.record(vec![primary("a:1", &[])], ok_health(), 5);
        assert!(d.drilled_node().is_none());
    }

    #[test]
    fn the_ops_history_records_the_summed_figure_per_poll() {
        let mut d = dash(vec![primary("a:1", &[("instantaneous_ops_per_sec", "4")])]);
        d.record(
            vec![primary("a:1", &[("instantaneous_ops_per_sec", "9")])],
            ok_health(),
            2,
        );
        assert_eq!(d.ops_history().iter().copied().collect::<Vec<_>>(), [4, 9]);
    }

    #[test]
    fn health_parses_and_reads_ok() {
        let h = ClusterHealth::parse(&ok_health().unwrap()).unwrap();
        assert_eq!(h.level(), AlarmLevel::Ok);
        assert_eq!(
            h.line("·"),
            "cluster_state ok · 16384/16384 slots · 0 failing"
        );
    }

    #[test]
    fn health_alarms_on_fail_unassigned_slots_or_failing_slots() {
        let fail =
            ClusterHealth::parse("cluster_state:fail\r\ncluster_slots_assigned:16384\r\n").unwrap();
        assert_eq!(fail.level(), AlarmLevel::Danger);
        let gap =
            ClusterHealth::parse("cluster_state:ok\r\ncluster_slots_assigned:16000\r\n").unwrap();
        assert_eq!(gap.level(), AlarmLevel::Danger);
        assert!(gap.line("·").contains("16000/16384"));
        let pfail = ClusterHealth::parse(
            "cluster_state:ok\r\ncluster_slots_assigned:16384\r\ncluster_slots_pfail:2\r\ncluster_slots_fail:1\r\n",
        )
        .unwrap();
        assert_eq!(pfail.failing(), 3);
        assert_eq!(pfail.level(), AlarmLevel::Danger);
    }

    #[test]
    fn text_without_a_cluster_state_is_not_health() {
        assert!(ClusterHealth::parse("redis_version:8.4.0\r\n").is_none());
        let mut d = ClusterDash::default();
        d.record(vec![primary("a:1", &[])], Ok("garbage".into()), 1);
        assert!(matches!(d.health(), Some(Err(_))));
    }

    #[test]
    fn a_failed_cluster_info_is_kept_as_the_error() {
        let mut d = ClusterDash::default();
        d.record(
            vec![primary("a:1", &[])],
            Err("CLUSTER INFO: boom".into()),
            1,
        );
        assert_eq!(d.health(), Some(&Err("CLUSTER INFO: boom".to_string())));
    }
}
