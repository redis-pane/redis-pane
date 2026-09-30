//! Dashboard state (`g d`, R6.3, M3 task 6, `docs/plans/m3-dashboard.md`):
//! the server's own vitals, parsed from `INFO` and polled on a shell-side
//! interval rather than pushed (decision 1 — the core never owns a timer).
//!
//! Mirrors the Slowlog view's split (`state/slowlog.rs`, `update/slowlog.rs`)
//! in shape: a plain, bounded state struct here, with `update/dashboard.rs`
//! owning the `g d`/refetch entry points and the two replies
//! `Command::FetchServerInfo` can answer with. What differs is *how* the data
//! is read: tiles are never stored, only derived. `DashboardState` holds the
//! raw ingredients (the parsed `INFO` reply, the previous poll's, the ops/sec
//! history, which tile is focused/expanded) and every tile — memory, hit
//! ratio, clients, replication, eviction — along with its alarm level is a
//! pure function of those ingredients, computed at render time. That is what
//! "anything alarming is colored" and "every tile expands to its raw `INFO`
//! section" can both be — a read of data already held, never a second fetch
//! or a value that can drift from what `raw` actually says.

use std::collections::VecDeque;

/// One `INFO` reply, parsed but unprocessed: section name (as `INFO` spells
/// it after `# `, e.g. `"Memory"`) to an ordered list of its `key:value`
/// pairs, in the order the server sent them. No `fred` type crosses this
/// boundary (ADR-0011) — this is plain, core-owned data, built shell-side by
/// `crates/app/src/redis/read.rs`'s `fetch_server_info`.
///
/// Ordered rather than a map, twice over: sections keep the server's own
/// order (so the raw-`INFO`-section drill-down reads exactly like `redis-cli
/// INFO` would), and so does each section's fields — a `Vec<(String,
/// String)>` line, not a `HashMap`, since this is at most a few hundred short
/// lines, never a hot path, and a stable order is worth more here than a
/// lookup that is already `O(n)` on a section this small.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawInfo {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl RawInfo {
    /// Build a `RawInfo` from already-split `(section, fields)` pairs —
    /// what `fetch_server_info`'s parser produces. Not `pub` beyond this
    /// crate boundary's own shell: the shell builds one exactly once, from
    /// the `INFO` text itself.
    pub fn new(sections: Vec<(String, Vec<(String, String)>)>) -> Self {
        Self { sections }
    }

    /// Every section, in the server's own order — what the raw-`INFO`
    /// overlay (phase B) iterates to render one section's drill-down.
    pub fn sections(&self) -> &[(String, Vec<(String, String)>)] {
        &self.sections
    }

    /// One section's fields, if the reply had it. Absent rather than a
    /// default: a fresh container with no replicas has no `Replication`
    /// oddities, but ACL-restricted `INFO` output can drop whole sections,
    /// and every tile function below has to tolerate that (decision 7's
    /// "never silently stale" starts with "never silently absent").
    pub fn section(&self, name: &str) -> Option<&[(String, String)]> {
        self.sections
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, fields)| fields.as_slice())
    }

    /// A field's raw value, wherever it lives. Most `INFO` field names are
    /// unique across sections in practice (the same convention
    /// `server_conditions` already relies on in `crates/app/src/redis/mod.rs`),
    /// and every tile function below wants "the value of this field" without
    /// first naming which section it happens to be filed under.
    pub fn field(&self, key: &str) -> Option<&str> {
        self.sections
            .iter()
            .flat_map(|(_, fields)| fields.iter())
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn field_u64(&self, key: &str) -> Option<u64> {
        self.field(key).and_then(|v| v.parse().ok())
    }

    fn field_i64(&self, key: &str) -> Option<i64> {
        self.field(key).and_then(|v| v.parse().ok())
    }
}

/// One `key=value` out of a primary's `slaveN:ip=...,port=...,state=online,
/// offset=...,lag=0` line — `INFO`'s own inline format for describing one
/// replica, distinct from the section's own `key:value` lines. No
/// allocation: a linear scan over a handful of comma-separated pairs, never
/// a hot path.
fn replica_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split(',').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then_some(v)
    })
}

/// How many polls the ops/sec sparkline keeps (~2 minutes at the 2s poll
/// interval, decision 1) — the same cap discipline Monitor's/Pub-Sub's
/// buffers use (`MONITOR_CAP`, `PUBSUB_CAP`), one poll wide instead of one
/// line wide. Named for the sparkline that reads it (phase B); enforced here,
/// in the one place a poll's ops/sec figure is ever pushed.
pub const OPS_HISTORY_CAP: usize = 60;

/// Memory-used-against-`maxmemory` alarm thresholds (decision 4). Named
/// constants, not literals scattered through the tile functions below — the
/// numbers this whole screen exists to catch belong in one place, with the
/// citation that fixed them.
pub const MEMORY_WARN_RATIO: f64 = 0.80;
pub const MEMORY_DANGER_RATIO: f64 = 0.95;

/// Hit-ratio alarm threshold and the sample floor below which it does not
/// apply (decision 4): a container that has answered four reads has no
/// meaningful hit ratio yet, and coloring it red on the first miss would be
/// noise, not signal.
pub const HIT_RATIO_WARN_RATIO: f64 = 0.80;
pub const HIT_RATIO_MIN_SAMPLES: u64 = 1_000;

/// Replication lag alarm thresholds, in seconds (decision 4).
pub const REPLICA_LAG_WARN_SECS: i64 = 5;
pub const REPLICA_LAG_DANGER_SECS: i64 = 30;

/// Where a value sits against its alarm thresholds. Deliberately not tied to
/// `crate::theme::Token` here: which semantic token paints `Warn`/`Danger`
/// is a render-layer decision (phase B); this is the pure judgement a tile
/// function reaches before any rendering exists to read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlarmLevel {
    Ok,
    Warn,
    Danger,
}

/// Which tile a keypress or an overlay names (decision 6's grid). Defined
/// here, ahead of the render it drives, so `DashboardState` has somewhere
/// real for `focused_tile`/`expanded_tile` to point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileId {
    #[default]
    Memory,
    HitRatio,
    Ops,
    Clients,
    Replication,
    Eviction,
}

/// The grid's own row-major order — top row first, left to right — which is
/// what `←→↑↓`/`hjkl` tile-focus movement walks (decision 6) and what the
/// render's grid (phase B) lays tiles out in. One canonical order, read by
/// both, so a reordered grid and a reordered movement path cannot drift
/// apart.
pub const TILE_ORDER: [TileId; 6] = [
    TileId::Memory,
    TileId::HitRatio,
    TileId::Ops,
    TileId::Clients,
    TileId::Replication,
    TileId::Eviction,
];

impl TileId {
    /// The `INFO` section this tile is drawn from — what `Enter`'s overlay
    /// shows verbatim (decision 6, "every tile expands to its raw `INFO`
    /// section"). Several tiles share `Stats`: `INFO` does not subdivide it
    /// any further, so their overlays show the same section, which is the
    /// literal reading of "the raw section behind it," not a narrower slice
    /// invented for this screen.
    pub fn section_name(&self) -> &'static str {
        match self {
            TileId::Memory => "Memory",
            TileId::HitRatio | TileId::Ops | TileId::Eviction => "Stats",
            TileId::Clients => "Clients",
            TileId::Replication => "Replication",
        }
    }

    /// The grid tile's own title (phase B render).
    pub fn label(&self) -> &'static str {
        match self {
            TileId::Memory => "MEMORY",
            TileId::HitRatio => "HIT RATIO",
            TileId::Ops => "OPS/SEC",
            TileId::Clients => "CLIENTS",
            TileId::Replication => "REPLICATION",
            TileId::Eviction => "EVICTIONS",
        }
    }
}

/// How many tiles the grid puts on one row (decision 5's breakpoints,
/// recorded in DESIGN.md §2): `≥120` columns is three, `80..119` is two,
/// below `80` is one and the grid scrolls. A pure function of `cols` alone
/// (ADR-0011: no geometry in the core beyond `cols`/`rows` themselves) —
/// shared by the render grid (phase B) and by tile-focus movement
/// (`DashboardState::focus_up`/`focus_down`), so the two cannot disagree
/// about how many tiles are on a row.
pub fn tiles_per_row(cols: u16) -> usize {
    if cols >= 120 {
        3
    } else if cols >= 80 {
        2
    } else {
        1
    }
}

/// Memory used vs. peak vs. `maxmemory`, as a bar (decision 3) —
/// `maxmemory == 0` means "no limit", not "no bar starting at zero"; a bar
/// with no ceiling has nothing to fill it against, and `level` is always
/// `Ok` in that case, since there is nothing to approach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryTile {
    pub used_bytes: u64,
    pub peak_bytes: u64,
    /// `None` when `maxmemory` is unset or `0`.
    pub max_bytes: Option<u64>,
    pub level: AlarmLevel,
}

/// `keyspace_hits` / (`keyspace_hits` + `keyspace_misses`) (decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRatioTile {
    pub hits: u64,
    pub misses: u64,
    pub level: AlarmLevel,
}

impl HitRatioTile {
    /// `None` with no samples yet — a ratio of zero would claim a 0% hit
    /// rate a fresh container has not actually earned.
    pub fn ratio(&self) -> Option<f64> {
        let total = self.hits + self.misses;
        (total > 0).then(|| self.hits as f64 / total as f64)
    }
}

/// Connected/blocked clients (decision 3). `rejected_connections` rising
/// between polls is folded into this tile's alarm (decision 4) rather than
/// given a seventh tile of its own — it is a fact about client admission,
/// the same subject this tile already covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientsTile {
    pub connected: u64,
    pub blocked: u64,
    pub level: AlarmLevel,
}

/// Replication role/lag/link status (decision 3). What `lag_secs` measures
/// depends on `role`, since `INFO` describes the two sides of a replication
/// link with entirely different fields:
///
/// - On a primary (`role:master`), it is the **worst** lag across every
///   `slaveN:...,lag=N` line — a primary with several replicas is only as
///   caught-up as its slowest one, and `INFO` reports each separately (no
///   single aggregate field exists to read instead). `link_status` is
///   always `None` here; `master_link_status` is a replica-side concept.
/// - On a replica (`role:slave`), it is `master_last_io_seconds_ago` — the
///   liveness proxy `INFO` actually offers for the link *to* the master;
///   there is no literal "seconds behind" field to read instead.
///   `link_status` is `master_link_status` (`"up"`/`"down"`).
///
/// `None`/`0` on a fresh container with no replication configured at all —
/// the tile still renders, just with nothing to alarm on (decision 3's own
/// "no replicas" case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationTile {
    pub role: String,
    pub connected_replicas: u64,
    pub lag_secs: Option<i64>,
    pub link_status: Option<String>,
    pub level: AlarmLevel,
}

/// Evictions and expiries (decision 3). `evicted_keys` rising between polls
/// is a `Warn` (decision 4) — `previous` is what makes that a comparison
/// rather than a bare count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionTile {
    pub evicted_keys: u64,
    pub expired_keys: u64,
    pub level: AlarmLevel,
}

/// The `g d` view's whole state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DashboardState {
    /// The most recent successful `INFO` parse. `None` before the first
    /// reply lands — the "no blank frame" requirement (decision 1) is met by
    /// issuing the fetch immediately on `g d`, not by this ever being
    /// synthesized.
    raw: Option<RawInfo>,
    /// The poll before `raw` — what the counter-based alarms (evictions,
    /// rejected connections) compare against. `None` before the second
    /// successful poll, which is also the only state in which those two
    /// alarms cannot yet fire (there is nothing to compare against on the
    /// first reading).
    previous: Option<RawInfo>,
    /// The clock reading (ADR-0011) at the last successful poll — what
    /// `updated Ns ago` renders from (phase B). Not advanced by a failed
    /// poll: the header must keep showing the age of the last *good* value
    /// (decision 7), never restart its clock on an error.
    pub last_updated_ms: Option<u64>,
    /// `instantaneous_ops_per_sec` at each successful poll, oldest first,
    /// bounded to [`OPS_HISTORY_CAP`] — the sparkline's own backing buffer
    /// (phase B renders it; this crate only ever bounds it).
    ops_history: VecDeque<u64>,
    /// Which tile the cursor is on — the grid's own focus (decision 6).
    pub focused_tile: TileId,
    /// The tile whose raw `INFO` section is expanded into an overlay, if
    /// any (decision 6's `Enter`/`Esc`).
    pub expanded_tile: Option<TileId>,
    /// How far the raw-`INFO` overlay has scrolled — decision 6 calls it
    /// "scrollable" (a section can run to dozens of lines, e.g. `Stats`).
    /// Reset to `0` whenever a different tile is expanded, so a long
    /// scroll on one section never carries over to the next tile opened.
    pub overlay_scroll: usize,
    /// The token of the most recently issued `Command::FetchServerInfo`
    /// (decision (b), `docs/plans/m3-dashboard.md` review) — what
    /// `Msg::ServerInfoLoaded`/`Msg::ServerInfoFailed` compare their own
    /// `token` against before applying. Minted only by
    /// `update::dashboard::issue_info_token`, never by a shell, the same
    /// discipline `State::read_token` follows for `ReadToken`.
    pub poll_token: crate::command::InfoToken,
    /// A fetch is in flight — set by `g d`/`r`/an accepted poll tick,
    /// cleared by `Msg::ServerInfoLoaded`/`Msg::ServerInfoFailed` (whichever
    /// carries the current `poll_token`), the same shape
    /// `SlowlogState::loading` already has. Also what
    /// `Msg::DashboardPollTick`'s own handling reads before firing another
    /// fetch — a tick that finds this still `true` is a no-op, so a slow
    /// reply cannot pile up requests.
    pub loading: bool,
    /// The last fetch's failure, if the most recent one failed — the
    /// in-view empty/stale state's own words, alongside the R7.4
    /// notification `State::error` separately carries (decision 7).
    pub error: Option<String>,
}

impl DashboardState {
    /// The raw parse behind every tile, and behind the raw-`INFO` overlay
    /// (phase B) — `None` before the first successful poll.
    pub fn raw(&self) -> Option<&RawInfo> {
        self.raw.as_ref()
    }

    /// Record a successful poll: rotate `raw` into `previous` (so the next
    /// poll's counter-based alarms have something to compare against),
    /// push this poll's ops/sec onto the bounded history, and clear
    /// whatever error the last poll left behind.
    ///
    /// The one place `ops_history` grows — the same "enforced in exactly one
    /// place" discipline `scan_batch` applies to the Loaded set's cap
    /// (ADR-0010), one buffer over.
    pub fn record_poll(&mut self, info: RawInfo, at_ms: u64) {
        let ops = info.field_u64("instantaneous_ops_per_sec").unwrap_or(0);
        self.previous = self.raw.take();
        self.raw = Some(info);
        self.last_updated_ms = Some(at_ms);
        self.loading = false;
        self.error = None;
        self.ops_history.push_back(ops);
        while self.ops_history.len() > OPS_HISTORY_CAP {
            self.ops_history.pop_front();
        }
    }

    /// The ops/sec history, oldest first, for the sparkline (phase B).
    pub fn ops_history(&self) -> &VecDeque<u64> {
        &self.ops_history
    }

    fn focus_index(&self) -> usize {
        TILE_ORDER
            .iter()
            .position(|&t| t == self.focused_tile)
            .unwrap_or(0)
    }

    /// `←`/`h`: move focus one tile left in the grid's flat, row-major
    /// order — clamped at the first tile, never wrapping to the last row
    /// (decision 6).
    pub fn focus_left(&mut self) {
        let i = self.focus_index();
        if i > 0 {
            self.focused_tile = TILE_ORDER[i - 1];
        }
    }

    /// `→`/`l`: the mirror of [`DashboardState::focus_left`].
    pub fn focus_right(&mut self) {
        let i = self.focus_index();
        if i + 1 < TILE_ORDER.len() {
            self.focused_tile = TILE_ORDER[i + 1];
        }
    }

    /// `↑`/`k`: move focus up one row of `tiles_per_row` tiles — a no-op
    /// already on the top row (decision 6).
    pub fn focus_up(&mut self, tiles_per_row: usize) {
        let tiles_per_row = tiles_per_row.max(1);
        let i = self.focus_index();
        if i >= tiles_per_row {
            self.focused_tile = TILE_ORDER[i - tiles_per_row];
        }
    }

    /// `↓`/`j`: the mirror of [`DashboardState::focus_up`] — a no-op past
    /// the last tile rather than wrapping to a row that does not exist.
    pub fn focus_down(&mut self, tiles_per_row: usize) {
        let tiles_per_row = tiles_per_row.max(1);
        let i = self.focus_index();
        let j = i + tiles_per_row;
        if j < TILE_ORDER.len() {
            self.focused_tile = TILE_ORDER[j];
        }
    }

    /// `Enter`: expand the focused tile's raw `INFO` section into the
    /// overlay (decision 6). Resets the scroll — opening a *different*
    /// tile must not inherit however far the last one had scrolled.
    pub fn open_overlay(&mut self) {
        self.expanded_tile = Some(self.focused_tile);
        self.overlay_scroll = 0;
    }

    /// `Esc` with the overlay open: close it without leaving the view
    /// (decision 6 — `Esc` only leaves the Dashboard once no overlay is
    /// open, the same "nearest thing first" rule every other overlay in
    /// this app follows).
    pub fn close_overlay(&mut self) {
        self.expanded_tile = None;
        self.overlay_scroll = 0;
    }

    /// The expanded tile's own section, for the overlay's render and for
    /// `c`'s copy — `None` when nothing is expanded, or before the first
    /// successful poll (there is no raw `INFO` yet to show a section of).
    pub fn expanded_section(&self) -> Option<(TileId, &[(String, String)])> {
        let tile = self.expanded_tile?;
        let raw = self.raw.as_ref()?;
        let fields = raw.section(tile.section_name()).unwrap_or(&[]);
        Some((tile, fields))
    }

    /// The memory tile, or `None` before the first successful poll.
    pub fn memory_tile(&self) -> Option<MemoryTile> {
        let raw = self.raw.as_ref()?;
        let used_bytes = raw.field_u64("used_memory").unwrap_or(0);
        let peak_bytes = raw.field_u64("used_memory_peak").unwrap_or(used_bytes);
        let max_bytes = raw.field_u64("maxmemory").filter(|&m| m > 0);
        let level = match max_bytes {
            None => AlarmLevel::Ok,
            Some(max) => {
                let ratio = used_bytes as f64 / max as f64;
                if ratio >= MEMORY_DANGER_RATIO {
                    AlarmLevel::Danger
                } else if ratio >= MEMORY_WARN_RATIO {
                    AlarmLevel::Warn
                } else {
                    AlarmLevel::Ok
                }
            }
        };
        Some(MemoryTile {
            used_bytes,
            peak_bytes,
            max_bytes,
            level,
        })
    }

    /// The hit-ratio tile, or `None` before the first successful poll.
    pub fn hit_ratio_tile(&self) -> Option<HitRatioTile> {
        let raw = self.raw.as_ref()?;
        let hits = raw.field_u64("keyspace_hits").unwrap_or(0);
        let misses = raw.field_u64("keyspace_misses").unwrap_or(0);
        let total = hits + misses;
        let level = if total >= HIT_RATIO_MIN_SAMPLES {
            let ratio = hits as f64 / total as f64;
            if ratio < HIT_RATIO_WARN_RATIO {
                AlarmLevel::Warn
            } else {
                AlarmLevel::Ok
            }
        } else {
            AlarmLevel::Ok
        };
        Some(HitRatioTile {
            hits,
            misses,
            level,
        })
    }

    /// The clients tile, or `None` before the first successful poll.
    /// `rejected_connections` rising between polls escalates this tile to
    /// `Danger` regardless of `blocked_clients` (decision 4: rejection is
    /// the more severe of the two facts this tile can alarm on).
    pub fn clients_tile(&self) -> Option<ClientsTile> {
        let raw = self.raw.as_ref()?;
        let connected = raw.field_u64("connected_clients").unwrap_or(0);
        let blocked = raw.field_u64("blocked_clients").unwrap_or(0);
        let rejected_now = raw.field_u64("rejected_connections");
        let rejected_before = self
            .previous
            .as_ref()
            .and_then(|p| p.field_u64("rejected_connections"));
        let rejections_rising = matches!(
            (rejected_before, rejected_now),
            (Some(before), Some(now)) if now > before
        );
        let level = if rejections_rising {
            AlarmLevel::Danger
        } else if blocked > 0 {
            AlarmLevel::Warn
        } else {
            AlarmLevel::Ok
        };
        Some(ClientsTile {
            connected,
            blocked,
            level,
        })
    }

    /// The replication tile, or `None` before the first successful poll.
    ///
    /// Branches on `role`, per [`ReplicationTile`]'s own doc comment: a
    /// primary's lag lives in its `slaveN:...` lines (one per replica,
    /// worst one wins, any non-`online` state is `Danger` outright); a
    /// replica's lives in `master_last_io_seconds_ago`/`master_link_status`.
    /// Anything else (no `Replication` section at all — an ACL-restricted
    /// `INFO`, or a role this app has never seen) falls through to the
    /// replica-shaped branch, which reads absent fields as `None` and comes
    /// back `Ok` — the same "tolerate, don't panic" the parser itself
    /// promises.
    pub fn replication_tile(&self) -> Option<ReplicationTile> {
        let raw = self.raw.as_ref()?;
        let role = raw.field("role").unwrap_or("unknown").to_string();
        if role == "master" {
            let connected_replicas = raw.field_u64("connected_slaves").unwrap_or(0);
            let mut worst_lag: Option<i64> = None;
            let mut any_not_online = false;
            for i in 0..connected_replicas {
                let Some(line) = raw.field(&format!("slave{i}")) else {
                    continue;
                };
                if replica_field(line, "state") != Some("online") {
                    any_not_online = true;
                }
                if let Some(lag) = replica_field(line, "lag").and_then(|v| v.parse::<i64>().ok()) {
                    worst_lag = Some(worst_lag.map_or(lag, |w: i64| w.max(lag)));
                }
            }
            let level = if any_not_online {
                AlarmLevel::Danger
            } else {
                match worst_lag {
                    Some(lag) if lag > REPLICA_LAG_DANGER_SECS => AlarmLevel::Danger,
                    Some(lag) if lag > REPLICA_LAG_WARN_SECS => AlarmLevel::Warn,
                    _ => AlarmLevel::Ok,
                }
            };
            Some(ReplicationTile {
                role,
                connected_replicas,
                lag_secs: worst_lag,
                link_status: None,
                level,
            })
        } else {
            let lag_secs = raw.field_i64("master_last_io_seconds_ago");
            let link_status = raw.field("master_link_status").map(|s| s.to_string());
            let link_down = link_status.as_deref() == Some("down");
            let level = if link_down {
                AlarmLevel::Danger
            } else {
                match lag_secs {
                    Some(lag) if lag > REPLICA_LAG_DANGER_SECS => AlarmLevel::Danger,
                    Some(lag) if lag > REPLICA_LAG_WARN_SECS => AlarmLevel::Warn,
                    _ => AlarmLevel::Ok,
                }
            };
            Some(ReplicationTile {
                role,
                connected_replicas: 0,
                lag_secs,
                link_status,
                level,
            })
        }
    }

    /// The eviction tile, or `None` before the first successful poll.
    /// `evicted_keys` rising between polls is what alarms it (decision 4) —
    /// a nonzero but flat count (evictions from before this session opened)
    /// is not itself a `Warn`.
    pub fn eviction_tile(&self) -> Option<EvictionTile> {
        let raw = self.raw.as_ref()?;
        let evicted_keys = raw.field_u64("evicted_keys").unwrap_or(0);
        let expired_keys = raw.field_u64("expired_keys").unwrap_or(0);
        let evicted_before = self
            .previous
            .as_ref()
            .and_then(|p| p.field_u64("evicted_keys"));
        let evictions_rising = matches!(evicted_before, Some(before) if evicted_keys > before);
        let level = if evictions_rising {
            AlarmLevel::Warn
        } else {
            AlarmLevel::Ok
        };
        Some(EvictionTile {
            evicted_keys,
            expired_keys,
            level,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real-looking Redis 7/8 `INFO` reply, trimmed to the sections and
    /// fields the tiles actually read — the fixture every tile-derivation
    /// test below starts from.
    fn fixture() -> RawInfo {
        RawInfo::new(vec![
            (
                "Clients".to_string(),
                vec![
                    ("connected_clients".to_string(), "12".to_string()),
                    ("blocked_clients".to_string(), "0".to_string()),
                ],
            ),
            (
                "Memory".to_string(),
                vec![
                    ("used_memory".to_string(), "500000".to_string()),
                    ("used_memory_peak".to_string(), "600000".to_string()),
                    ("maxmemory".to_string(), "1000000".to_string()),
                ],
            ),
            (
                "Stats".to_string(),
                vec![
                    ("instantaneous_ops_per_sec".to_string(), "42".to_string()),
                    ("keyspace_hits".to_string(), "9000".to_string()),
                    ("keyspace_misses".to_string(), "1000".to_string()),
                    ("evicted_keys".to_string(), "0".to_string()),
                    ("expired_keys".to_string(), "5".to_string()),
                    ("rejected_connections".to_string(), "0".to_string()),
                ],
            ),
            (
                "Replication".to_string(),
                vec![
                    ("role".to_string(), "master".to_string()),
                    ("connected_slaves".to_string(), "1".to_string()),
                    (
                        "slave0".to_string(),
                        "ip=127.0.0.1,port=6380,state=online,offset=196,lag=0".to_string(),
                    ),
                ],
            ),
        ])
    }

    fn replica_fixture(lag: i64, link_status: &str) -> RawInfo {
        RawInfo::new(vec![(
            "Replication".to_string(),
            vec![
                ("role".to_string(), "slave".to_string()),
                ("master_last_io_seconds_ago".to_string(), lag.to_string()),
                ("master_link_status".to_string(), link_status.to_string()),
            ],
        )])
    }

    #[test]
    fn every_tile_is_none_before_the_first_poll() {
        let s = DashboardState::default();
        assert!(s.memory_tile().is_none());
        assert!(s.hit_ratio_tile().is_none());
        assert!(s.clients_tile().is_none());
        assert!(s.replication_tile().is_none());
        assert!(s.eviction_tile().is_none());
    }

    #[test]
    fn record_poll_fills_last_updated_and_clears_loading_and_error() {
        let mut s = DashboardState {
            loading: true,
            error: Some("stale".into()),
            ..Default::default()
        };
        s.record_poll(fixture(), 1_000);
        assert_eq!(s.last_updated_ms, Some(1_000));
        assert!(!s.loading);
        assert!(s.error.is_none());
    }

    #[test]
    fn memory_tile_reads_used_peak_and_max() {
        let mut s = DashboardState::default();
        s.record_poll(fixture(), 0);
        let tile = s.memory_tile().unwrap();
        assert_eq!(tile.used_bytes, 500_000);
        assert_eq!(tile.peak_bytes, 600_000);
        assert_eq!(tile.max_bytes, Some(1_000_000));
    }

    #[test]
    fn memory_maxmemory_zero_means_no_limit_and_no_alarm() {
        let mut raw = fixture();
        raw = RawInfo::new(
            raw.sections()
                .iter()
                .map(|(name, fields)| {
                    if name == "Memory" {
                        (
                            name.clone(),
                            fields
                                .iter()
                                .map(|(k, v)| {
                                    if k == "maxmemory" {
                                        (k.clone(), "0".to_string())
                                    } else {
                                        (k.clone(), v.clone())
                                    }
                                })
                                .collect(),
                        )
                    } else {
                        (name.clone(), fields.clone())
                    }
                })
                .collect(),
        );
        let mut s = DashboardState::default();
        s.record_poll(raw, 0);
        let tile = s.memory_tile().unwrap();
        assert_eq!(tile.max_bytes, None);
        assert_eq!(tile.level, AlarmLevel::Ok);
    }

    #[test]
    fn memory_alarm_thresholds_at_and_just_past_the_boundary() {
        let cases = [
            (799_999u64, AlarmLevel::Ok),
            (800_000, AlarmLevel::Warn),
            (800_001, AlarmLevel::Warn),
            (949_999, AlarmLevel::Warn),
            (950_000, AlarmLevel::Danger),
            (950_001, AlarmLevel::Danger),
        ];
        for (used, expected) in cases {
            let raw = RawInfo::new(vec![(
                "Memory".to_string(),
                vec![
                    ("used_memory".to_string(), used.to_string()),
                    ("used_memory_peak".to_string(), used.to_string()),
                    ("maxmemory".to_string(), "1000000".to_string()),
                ],
            )]);
            let mut s = DashboardState::default();
            s.record_poll(raw, 0);
            assert_eq!(s.memory_tile().unwrap().level, expected, "used={used}");
        }
    }

    #[test]
    fn hit_ratio_needs_a_thousand_samples_before_it_can_warn() {
        // 799/999 misses badly (< 80%) but total is under the floor — no
        // alarm yet, exactly the "four reads on a fresh container" case the
        // floor exists for.
        let raw = RawInfo::new(vec![(
            "Stats".to_string(),
            vec![
                ("keyspace_hits".to_string(), "200".to_string()),
                ("keyspace_misses".to_string(), "799".to_string()),
            ],
        )]);
        let mut s = DashboardState::default();
        s.record_poll(raw, 0);
        let tile = s.hit_ratio_tile().unwrap();
        assert!(tile.ratio().unwrap() < 0.80);
        assert_eq!(tile.level, AlarmLevel::Ok, "under the 1000-sample floor");
    }

    #[test]
    fn hit_ratio_alarm_thresholds_at_and_just_past_the_boundary() {
        // Exactly 1000 samples, ratio exactly on and either side of 80%.
        let cases = [
            (801u64, 199u64, AlarmLevel::Ok), // 80.1%
            (800, 200, AlarmLevel::Ok),       // exactly 80% — "< 80%" does not warn
            (799, 201, AlarmLevel::Warn),     // 79.9%
        ];
        for (hits, misses, expected) in cases {
            let raw = RawInfo::new(vec![(
                "Stats".to_string(),
                vec![
                    ("keyspace_hits".to_string(), hits.to_string()),
                    ("keyspace_misses".to_string(), misses.to_string()),
                ],
            )]);
            let mut s = DashboardState::default();
            s.record_poll(raw, 0);
            assert_eq!(
                s.hit_ratio_tile().unwrap().level,
                expected,
                "hits={hits} misses={misses}"
            );
        }
    }

    #[test]
    fn hit_ratio_with_no_samples_at_all_is_ok_and_ratio_is_none() {
        let raw = RawInfo::new(vec![(
            "Stats".to_string(),
            vec![
                ("keyspace_hits".to_string(), "0".to_string()),
                ("keyspace_misses".to_string(), "0".to_string()),
            ],
        )]);
        let mut s = DashboardState::default();
        s.record_poll(raw, 0);
        let tile = s.hit_ratio_tile().unwrap();
        assert_eq!(tile.ratio(), None);
        assert_eq!(tile.level, AlarmLevel::Ok);
    }

    #[test]
    fn blocked_clients_above_zero_warns() {
        let cases = [
            (0u64, AlarmLevel::Ok),
            (1, AlarmLevel::Warn),
            (5, AlarmLevel::Warn),
        ];
        for (blocked, expected) in cases {
            let raw = RawInfo::new(vec![(
                "Clients".to_string(),
                vec![
                    ("connected_clients".to_string(), "3".to_string()),
                    ("blocked_clients".to_string(), blocked.to_string()),
                ],
            )]);
            let mut s = DashboardState::default();
            s.record_poll(raw, 0);
            assert_eq!(
                s.clients_tile().unwrap().level,
                expected,
                "blocked={blocked}"
            );
        }
    }

    #[test]
    fn rejected_connections_only_alarms_once_it_rises_between_polls() {
        let raw_flat = |rejected: u64| {
            RawInfo::new(vec![(
                "Stats".to_string(),
                vec![("rejected_connections".to_string(), rejected.to_string())],
            )])
        };
        let mut s = DashboardState::default();
        // First poll: nothing to compare against, so no alarm however high
        // the absolute count is.
        s.record_poll(raw_flat(50), 0);
        assert_eq!(s.clients_tile().unwrap().level, AlarmLevel::Ok);
        // Second poll, unchanged: still flat, still no alarm.
        s.record_poll(raw_flat(50), 1);
        assert_eq!(s.clients_tile().unwrap().level, AlarmLevel::Ok);
        // Third poll, risen: danger.
        s.record_poll(raw_flat(51), 2);
        assert_eq!(s.clients_tile().unwrap().level, AlarmLevel::Danger);
    }

    #[test]
    fn evicted_keys_only_warns_once_it_rises_between_polls() {
        let raw_flat = |evicted: u64| {
            RawInfo::new(vec![(
                "Stats".to_string(),
                vec![("evicted_keys".to_string(), evicted.to_string())],
            )])
        };
        let mut s = DashboardState::default();
        s.record_poll(raw_flat(10), 0);
        assert_eq!(s.eviction_tile().unwrap().level, AlarmLevel::Ok);
        s.record_poll(raw_flat(10), 1);
        assert_eq!(s.eviction_tile().unwrap().level, AlarmLevel::Ok);
        s.record_poll(raw_flat(11), 2);
        assert_eq!(s.eviction_tile().unwrap().level, AlarmLevel::Warn);
    }

    #[test]
    fn replication_with_no_section_at_all_is_none_not_a_panic() {
        let raw = RawInfo::new(vec![("Server".to_string(), vec![])]);
        let mut s = DashboardState::default();
        s.record_poll(raw, 0);
        let tile = s.replication_tile().unwrap();
        assert_eq!(tile.role, "unknown");
        assert_eq!(tile.connected_replicas, 0);
        assert_eq!(tile.lag_secs, None);
        assert_eq!(tile.level, AlarmLevel::Ok);
    }

    #[test]
    fn replication_lag_alarm_thresholds_at_and_just_past_the_boundary() {
        let cases = [
            (5i64, "up", AlarmLevel::Ok),
            (6, "up", AlarmLevel::Warn),
            (30, "up", AlarmLevel::Warn),
            (31, "up", AlarmLevel::Danger),
        ];
        for (lag, link, expected) in cases {
            let mut s = DashboardState::default();
            s.record_poll(replica_fixture(lag, link), 0);
            assert_eq!(s.replication_tile().unwrap().level, expected, "lag={lag}");
        }
    }

    #[test]
    fn replication_link_down_is_always_danger_regardless_of_lag() {
        let mut s = DashboardState::default();
        s.record_poll(replica_fixture(0, "down"), 0);
        assert_eq!(s.replication_tile().unwrap().level, AlarmLevel::Danger);
    }

    fn primary_fixture(replicas: &[(&str, i64)]) -> RawInfo {
        let mut fields = vec![
            ("role".to_string(), "master".to_string()),
            ("connected_slaves".to_string(), replicas.len().to_string()),
        ];
        for (i, (state, lag)) in replicas.iter().enumerate() {
            fields.push((
                format!("slave{i}"),
                format!("ip=127.0.0.1,port=638{i},state={state},offset=1,lag={lag}"),
            ));
        }
        RawInfo::new(vec![("Replication".to_string(), fields)])
    }

    #[test]
    fn a_primary_with_zero_replicas_is_ok_and_reports_no_lag() {
        let mut s = DashboardState::default();
        s.record_poll(primary_fixture(&[]), 0);
        let tile = s.replication_tile().unwrap();
        assert_eq!(tile.role, "master");
        assert_eq!(tile.connected_replicas, 0);
        assert_eq!(tile.lag_secs, None);
        assert_eq!(tile.level, AlarmLevel::Ok);
    }

    #[test]
    fn a_primary_reports_the_worst_lag_across_its_replicas() {
        let mut s = DashboardState::default();
        s.record_poll(primary_fixture(&[("online", 2), ("online", 40)]), 0);
        let tile = s.replication_tile().unwrap();
        assert_eq!(tile.connected_replicas, 2);
        assert_eq!(tile.lag_secs, Some(40), "the worse of the two replicas");
        assert_eq!(
            tile.level,
            AlarmLevel::Danger,
            "40s is past the danger threshold"
        );
    }

    #[test]
    fn a_primary_with_one_lagging_replica_warns() {
        let mut s = DashboardState::default();
        s.record_poll(primary_fixture(&[("online", 10)]), 0);
        let tile = s.replication_tile().unwrap();
        assert_eq!(tile.lag_secs, Some(10));
        assert_eq!(tile.level, AlarmLevel::Warn);
    }

    #[test]
    fn a_primary_with_a_replica_not_online_is_danger_regardless_of_lag() {
        let mut s = DashboardState::default();
        s.record_poll(primary_fixture(&[("online", 0), ("connect", 0)]), 0);
        let tile = s.replication_tile().unwrap();
        assert_eq!(tile.level, AlarmLevel::Danger);
    }

    // ── tile-focus movement and the raw-`INFO` overlay (decision 6) ─────────

    #[test]
    fn tiles_per_row_follows_the_documented_breakpoints() {
        assert_eq!(tiles_per_row(60), 1);
        assert_eq!(tiles_per_row(79), 1);
        assert_eq!(tiles_per_row(80), 2);
        assert_eq!(tiles_per_row(119), 2);
        assert_eq!(tiles_per_row(120), 3);
        assert_eq!(tiles_per_row(200), 3);
    }

    #[test]
    fn left_right_move_through_the_flat_grid_order_and_clamp_at_both_ends() {
        let mut s = DashboardState::default();
        assert_eq!(s.focused_tile, TileId::Memory);
        s.focus_left();
        assert_eq!(s.focused_tile, TileId::Memory, "clamped at the first tile");
        for expected in [
            TileId::HitRatio,
            TileId::Ops,
            TileId::Clients,
            TileId::Replication,
            TileId::Eviction,
        ] {
            s.focus_right();
            assert_eq!(s.focused_tile, expected);
        }
        s.focus_right();
        assert_eq!(s.focused_tile, TileId::Eviction, "clamped at the last tile");
    }

    #[test]
    fn up_down_move_by_a_full_row_at_the_given_width() {
        let mut s = DashboardState::default();
        s.focus_down(3); // Memory -> Clients, three tiles per row
        assert_eq!(s.focused_tile, TileId::Clients);
        s.focus_down(3); // past the grid: no-op
        assert_eq!(s.focused_tile, TileId::Clients);
        s.focus_up(3);
        assert_eq!(s.focused_tile, TileId::Memory);
    }

    #[test]
    fn enter_expands_the_focused_tile_and_esc_closes_it() {
        let mut s = DashboardState::default();
        s.record_poll(fixture(), 0);
        s.focused_tile = TileId::Replication;
        s.overlay_scroll = 7;
        s.open_overlay();
        assert_eq!(s.expanded_tile, Some(TileId::Replication));
        assert_eq!(s.overlay_scroll, 0, "opening a tile resets the scroll");
        let (tile, fields) = s.expanded_section().unwrap();
        assert_eq!(tile, TileId::Replication);
        assert!(fields.iter().any(|(k, _)| k == "role"));
        s.close_overlay();
        assert_eq!(s.expanded_tile, None);
        assert!(s.expanded_section().is_none());
    }

    #[test]
    fn ops_history_is_bounded_at_the_cap_across_many_pushes() {
        let mut s = DashboardState::default();
        for i in 0..(OPS_HISTORY_CAP as u64 * 3) {
            let raw = RawInfo::new(vec![(
                "Stats".to_string(),
                vec![("instantaneous_ops_per_sec".to_string(), i.to_string())],
            )]);
            s.record_poll(raw, i);
        }
        assert_eq!(s.ops_history().len(), OPS_HISTORY_CAP);
        // Oldest-first: the last value pushed is at the back.
        assert_eq!(
            *s.ops_history().back().unwrap(),
            OPS_HISTORY_CAP as u64 * 3 - 1
        );
    }

    #[test]
    fn a_fresh_container_fixture_populates_every_tile_without_panicking() {
        let mut s = DashboardState::default();
        s.record_poll(fixture(), 0);
        assert!(s.memory_tile().is_some());
        assert!(s.hit_ratio_tile().is_some());
        assert!(s.clients_tile().is_some());
        assert!(s.replication_tile().is_some());
        assert!(s.eviction_tile().is_some());
    }
}
