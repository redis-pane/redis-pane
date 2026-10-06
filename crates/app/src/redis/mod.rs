//! The Redis shell: connection, capability probe, and tracking (PLAN M0.8–M0.10).
//!
//! The server floor is RESP3 and Redis 6.0 (ADR-0007). Liveness is gated by
//! *capability*, never by version — managed platforms refuse `CLIENT TRACKING`
//! independently of the version they report, so ElastiCache Serverless will
//! answer `unknown subcommand 'tracking'` on an otherwise-current server.
//!
//! Two re-arm invariants hold here, both verified against Redis 8.4.0 (ADR-0006):
//!
//! 1. Every Refetch re-arms, because tracking is consumed by the invalidation
//!    it produces.
//! 2. Every reconnect re-arms before anything claims to be live.
//!
//! The core enforces the *claim* side of both — [`redis_pane_core::State::liveness`]
//! cannot return `Live` without an arming having been reported. This module is
//! responsible for the other half: actually doing it.

pub mod feed;
pub mod mutate;
pub mod read;
pub mod scan;

use std::time::Duration;

use fred::interfaces::{ClusterInterface, TrackingInterface};
use fred::prelude::*;
use fred::types::config::TlsConnector;
use fred::types::{InfoKind, RespVersion};
use redis_pane_core::msg::MetadataEntry;
use redis_pane_core::resolve::Credentials;
use redis_pane_core::server::{FLOOR, Version};
use redis_pane_core::state::{ReadOnlyReason, ServerCondition, Topology};

/// Why a connection could not be established or kept.
#[derive(Debug)]
pub enum ConnectError {
    /// The target could not be reached, or refused our credentials.
    Unreachable(String),
    /// Connected, but the server is below the floor (ADR-0007).
    BelowFloor { found: Version },
    /// The server rejected `HELLO 3`, so it does not speak RESP3 at all.
    ///
    /// In practice this, not [`ConnectError::BelowFloor`], is how an old server
    /// is caught: `HELLO` arrived *in* Redis 6.0, so protocol negotiation
    /// already excludes everything under the floor. The version check remains
    /// as defence in depth for a server that speaks RESP3 and still reports an
    /// older version, but this is the variant a user on Redis 5 actually sees —
    /// which is why it must not leak the raw protocol error.
    NoResp3 { detail: String },
    /// Connected, but `INFO server` did not report a parseable version.
    UnknownVersion(String),
    /// A password reference could not be resolved (ADR-0002).
    Credentials(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(why) => write!(f, "{why}"),
            ConnectError::BelowFloor { found } => write!(
                f,
                "server is Redis {found}; redis-pane needs {FLOOR} or newer.\n\
                 RESP3 is required and is not available before {FLOOR}."
            ),
            ConnectError::NoResp3 { detail } => write!(
                f,
                "server rejected RESP3 (HELLO 3), so it predates Redis {FLOOR}.\n\
                 redis-pane speaks RESP3 only and needs {FLOOR} or newer.\n\
                 The server said: {detail}"
            ),
            ConnectError::Credentials(why) => write!(f, "{why}"),
            ConnectError::UnknownVersion(raw) => {
                write!(
                    f,
                    "could not read the server version (INFO server said {raw:?})"
                )
            }
        }
    }
}

/// What a successful connect established.
#[derive(Debug, Clone)]
pub struct Established {
    pub version: Version,
    /// The result of *attempting* `CLIENT TRACKING`, never an inference.
    pub tracking_supported: bool,
    /// Set when the server reported `role:slave`. Read-only Mode goes on with
    /// reason `replica`, and `⌃R` cannot lift it (R1.15, ADR-0009).
    pub read_only: Option<ReadOnlyReason>,
    /// A condition currently rejecting writes.
    pub condition: Option<ServerCondition>,
    /// The replica and write-rejection checks could not run (INFO failed): the
    /// reason, shown to the reader rather than silently treated as a primary
    /// (review M1).
    pub server_state_error: Option<String>,
    /// The Cluster's shape, read from the client's routing table; `None` when
    /// the target is not a Cluster (M5 task 2).
    pub topology: Option<Topology>,
}

/// Connect over RESP3, check the floor, then probe for tracking. Dials a
/// Cluster as a cluster client (ADR-0022).
pub async fn connect(url: &str) -> Result<(Client, Established), ConnectError> {
    connect_with(url, &Credentials::default()).await
}

/// The Cluster's shape from `fred`'s cached routing table, or `None` when the
/// client is not a cluster client or has no table yet. Core does no I/O, so the
/// shell reads this and sends the result as a `Msg`.
pub fn topology_of(client: &Client) -> Option<Topology> {
    if !client.is_clustered() {
        return None;
    }
    let routing = client.cached_cluster_state()?;
    let mut primaries = std::collections::BTreeSet::new();
    let mut nodes = std::collections::BTreeSet::new();
    for range in routing.slots() {
        primaries.insert(range.primary.clone());
        nodes.insert(range.primary.clone());
        nodes.extend(range.replicas.iter().cloned());
    }
    Some(Topology {
        primaries: u16::try_from(primaries.len()).unwrap_or(u16::MAX),
        nodes: u16::try_from(nodes.len()).unwrap_or(u16::MAX),
    })
}

/// The primary that owns `slot` in the client's cached routing table, or
/// `None` when the client is not a cluster client or has no table yet.
///
/// The Viewer records this at every arming (M5 task 4, ADR-0022): the owner's
/// connection is the only one that holds the arming, so it is the only one
/// whose reconnect concerns the open key.
pub fn slot_owner(client: &Client, slot: u16) -> Option<fred::types::config::Server> {
    if !client.is_clustered() {
        return None;
    }
    client.cached_cluster_state()?.get_server(slot).cloned()
}

/// Build a `fred` `Config` for the resolved target, credentials and all.
///
/// The one place a URL and a `Credentials` become a dial-ready `Config` —
/// [`connect_with`] uses it for the main connection, and
/// [`crate::redis::feed::open_feed`] uses it for a second one, so a feed
/// connection resolves credentials through the exact path `connect_with`
/// does rather than re-parsing flags or, worse, falling back to the
/// credential-less [`connect`] (`docs/plans/m3-feed-connection.md`: "the same
/// credential path `connect_with` already resolves").
pub(crate) fn build_config(url: &str, credentials: &Credentials) -> Result<Config, ConnectError> {
    // Redacted, because this string is printed. A URL that fails to parse is
    // exactly the one someone pasted by hand with a real password in it, and
    // the next thing they do with a startup diagnostic is paste it into a bug
    // report. Everywhere else the dial URL and the displayable target are kept
    // deliberately apart; this was the one place they met.
    let mut config = Config::from_url(url).map_err(|e| {
        ConnectError::Unreachable(format!("{}: {e}", redis_pane_core::resolve::redact(url)))
    })?;

    // A Profile's credentials are the more specific statement of intent, so
    // they win over anything embedded in the URL.
    let password = crate::secret::resolve(&credentials.password)
        .map_err(|e| ConnectError::Credentials(e.to_string()))?;
    if password.is_some() {
        config.password = password;
    }
    if credentials.username.is_some() {
        config.username = credentials.username.clone();
    }
    // Managed Redis is TLS-only in practice — Upstash, Redis Cloud, Azure, and
    // ElastiCache in transit-encryption mode all refuse plaintext. A `rediss://`
    // URL already sets this; a Profile's `tls: true` is the other way to ask.
    if credentials.tls && config.tls.is_none() {
        config.tls = Some(
            TlsConnector::default_rustls()
                .map_err(|e| ConnectError::Unreachable(format!("TLS unavailable: {e}")))?
                .into(),
        );
    }
    // RESP2 is not spoken at all (ADR-0007): one reply shape per command, and
    // one code path per Viewer.
    config.version = RespVersion::RESP3;
    Ok(config)
}

/// Connect, authenticating with the credentials a Profile or the environment
/// supplied.
///
/// Without this, a Profile's `passwordEnv` is parsed, validated, and then
/// dropped — which makes the whole config file work on localhost and nowhere
/// else (ADR-0002).
pub async fn connect_with(
    url: &str,
    credentials: &Credentials,
) -> Result<(Client, Established), ConnectError> {
    let config = build_config(url, credentials)?;
    let (client, established, clustered) = establish(config).await?;
    if !clustered || client.is_clustered() {
        return Ok((client, established));
    }
    // A plain `redis://` URL that names a cluster node: every node reports
    // `cluster_enabled:1` whichever scheme dialled it (ADR-0022).
    // Redial as a cluster client to the same seed, with the same credentials
    // and TLS, because `build_config` produced them (M5 task 2, decision 2).
    let mut redial = build_config(url, credentials)?;
    if let ServerConfig::Centralized { server } = &redial.server {
        redial.server = ServerConfig::Clustered {
            hosts: vec![server.clone()],
            policy: Default::default(),
        };
    }
    let url_redacted = redis_pane_core::resolve::redact(url);
    match establish(redial).await {
        Ok((cluster_client, cluster_established, _)) => {
            let _ = client.quit().await;
            Ok((cluster_client, cluster_established))
        }
        // The first dial proved the node is reachable and cluster-enabled, so a
        // failed redial is the cluster-specific failure; say so in front of it.
        Err(ConnectError::Unreachable(why)) => Err(ConnectError::Unreachable(format!(
            "{url_redacted} is a Redis Cluster node, but dialling it as a Cluster failed: {why}"
        ))),
        Err(other) => Err(other),
    }
}

/// Dial `config`, check the floor and probe. The third value is whether `INFO`
/// reported `cluster_enabled:1`; the caller decides what to do about it.
async fn establish(config: Config) -> Result<(Client, Established, bool), ConnectError> {
    let mut builder = Builder::from_config(config);
    // Bound how long a silently dead socket can hide, on both the connection
    // and the command. Neither timing default fred ships is enough on its
    // own for a laptop-sleep/idle-NAT drop: a black-holed socket returns no
    // RST and no FIN, so `poll_next` just stays `Pending` forever, and
    // nothing here relies on the OS ever noticing.
    //
    // - `unresponsive.max_timeout` (`ConnectionConfig`) is fred's own dead-
    //   connection detector: a periodic check (`unresponsive.interval`, kept
    //   at fred's 2s default) that force-closes a connection which has had a
    //   write outstanding longer than this without a reply, then reconnects
    //   it. It surfaces as `ErrorKind::IO` on `error_rx`
    //   (`fred-10.1.0/src/router/types.rs:58`) — see `Shell::watch_link`.
    // - `default_command_timeout` (`PerformanceConfig`) is a per-command
    //   backstop that fires client-side, independent of whether fred's own
    //   connection machinery ever notices the socket is dead. This is what
    //   actually guarantees `read.await` inside `ReadPermit::run`
    //   (`crates/app/src/redis/read.rs`) resolves — the held mutex is only
    //   ever released once that `.await` returns, Ok or Err — and it
    //   produces `ErrorKind::Timeout`
    //   (`fred-10.1.0/src/utils.rs:286-299`, `fred-10.1.0/src/types/config.rs`
    //   `Command::inherit_options`). Set comfortably above the connection
    //   timeout so the connection-level detector gets first chance to
    //   classify the failure correctly; this is the backstop, not the
    //   primary signal.
    //
    // Both values are generous rather than snappy on purpose: this product
    // is explicitly meant to run over SSH to a bastion host (CLAUDE.md), and
    // a fast timeout would misclassify ordinary WAN latency as a dead
    // connection.
    builder
        .with_connection_config(|connection| {
            connection.unresponsive.max_timeout = Some(Duration::from_secs(6));
        })
        .with_performance_config(|performance| {
            performance.default_command_timeout = Duration::from_secs(10);
        });

    let client = builder
        .build()
        .map_err(|e| ConnectError::Unreachable(e.to_string()))?;
    client.init().await.map_err(|e| {
        let detail = describe(&e);
        // Turn an unhelpful protocol error into the diagnostic the user needs.
        if rejected_hello(&detail) {
            ConnectError::NoResp3 { detail }
        } else {
            ConnectError::Unreachable(detail)
        }
    })?;

    let version = server_version(&client).await?;
    if !version.meets_floor() {
        return Err(ConnectError::BelowFloor { found: version });
    }

    let tracking_supported = probe_tracking(&client).await;
    // A refused INFO must not fail the connection — it would turn an ACL
    // restriction into an outage. It also must not be swallowed: silently
    // defaulting `read_only`/`condition` to `None` here is exactly how a
    // replica gets treated as a primary, so the failure is carried forward
    // instead and surfaced by every caller (review M1).
    //
    // The Cluster check (`INFO`'s `cluster_enabled:1`) rides this same read —
    // `fetch_conditions` is the one place `INFO server` is parsed for this
    // purpose, so no second round trip is added (ADR-0022). This catches a
    // Cluster target reached by a plain `redis://`/`rediss://` URL, which the
    // scheme check in `build_config` cannot — every node of a Cluster reports
    // this regardless of which scheme dialled it.
    let (read_only, condition, server_state_error, clustered) =
        match fetch_conditions(&client).await {
            Ok((read_only, condition, clustered)) => (read_only, condition, None, clustered),
            Err(e) => (None, None, Some(describe(&e)), false),
        };
    let topology = topology_of(&client);
    Ok((
        client,
        Established {
            version,
            tracking_supported,
            read_only,
            condition,
            server_state_error,
            topology,
        },
        clustered,
    ))
}

/// Detect the conditions that will reject writes (R1.15, ADR-0009).
///
/// Detected rather than merely reported, so danger is visible *before* it is
/// possible. Learning that a server is a replica by having a write rejected is
/// exactly the ordering DESIGN principle 5 forbids.
pub async fn server_conditions(
    client: &Client,
) -> Result<(Option<ReadOnlyReason>, Option<ServerCondition>), Error> {
    let (read_only, condition, _clustered) = fetch_conditions(client).await?;
    Ok((read_only, condition))
}

/// `server_conditions`'s own fetch and parse, plus whether `INFO` reports
/// Cluster mode — one `INFO` read shared by both, so the Cluster check added
/// by ADR-0022 does not cost a second round trip. `server_conditions` stays
/// the public, Cluster-unaware entry point `terminal.rs`'s reconnect path
/// already calls; `connect_with` calls this directly so it can see the
/// `clustered` flag too.
async fn fetch_conditions(
    client: &Client,
) -> Result<(Option<ReadOnlyReason>, Option<ServerCondition>, bool), Error> {
    // Propagated, not defaulted: an ACL without `info` makes this `Err`, and
    // defaulting to an empty string here is exactly how a replica went
    // unnoticed and got treated as a primary (review M1).
    let info: String = client.info(Some(InfoKind::Default)).await?;

    let field = |name: &str| -> Option<String> {
        info.lines()
            .find_map(|l| l.strip_prefix(name))
            .map(|v| v.trim().to_string())
    };

    // A cluster client sends a keyless `INFO` to whichever node it picks, so
    // one replica answering would make a healthy Cluster read as a replica.
    // On a Cluster the reason is a property of the whole Cluster instead
    // (`cluster_is_all_replica`, M5 task 6): writes go to primaries, so it is
    // locked as `replica` only when no reachable node is one.
    let replica = if client.is_clustered() {
        let nodes: String = client.cluster_nodes().await?;
        cluster_is_all_replica(&nodes)
    } else {
        matches!(field("role:").as_deref(), Some("slave") | Some("replica"))
    };
    let read_only = replica.then_some(ReadOnlyReason::Replica);

    // A failing background save makes Redis refuse writes with -MISCONF, and it
    // stays broken until someone intervenes — worth a banner, not a surprise.
    let misconf = matches!(field("rdb_last_bgsave_status:").as_deref(), Some("err"));
    let used: u64 = field("used_memory:")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let max: u64 = field("maxmemory:")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let condition = if misconf {
        Some(ServerCondition::Misconf)
    } else if max > 0 && used >= max {
        Some(ServerCondition::Oom)
    } else if field("loading:").as_deref() == Some("1") {
        Some(ServerCondition::Loading { percent: 0 })
    } else {
        None
    };

    // Both `INFO server`'s `redis_mode:cluster` and `INFO cluster`'s
    // `cluster_enabled:1` report Cluster mode (confirmed against a real
    // `redis:7-alpine --cluster-enabled yes` container, 2026-10-03); both are
    // present in the `InfoKind::Default` reply already fetched above, so
    // checking either adds no extra command. `cluster_enabled:1` is used
    // because it is the literal boolean flag ADR-0022 and `monitor_config`'s
    // existing check (`config.server.is_clustered()`) already talk about.
    let clustered = field("cluster_enabled:").as_deref() == Some("1");

    Ok((read_only, condition, clustered))
}

/// Whether a Cluster is replica-only: every *reachable* node in `CLUSTER NODES`
/// is a replica, so there is nowhere for a write to land (M5 task 6, ADR-0022).
///
/// `CLUSTER NODES` rather than the client's routing table: that table is built
/// from `CLUSTER SLOTS`, whose entries are primaries by construction, so it can
/// never say "no primary". Nodes flagged `fail`, `fail?`, `handshake` or
/// `noaddr`, or whose link is down, are not reachable and do not count; the
/// answering node (`myself`) always does. No reachable node at all is not "all
/// replicas": that is an outage, which the connection reports by failing, not by
/// a Read-only Mode reason that could never lift.
pub fn cluster_is_all_replica(cluster_nodes: &str) -> bool {
    let mut primaries = 0usize;
    let mut replicas = 0usize;
    for line in cluster_nodes.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 {
            continue;
        }
        let flags: Vec<&str> = f[2].split(',').collect();
        let has = |x: &str| flags.contains(&x);
        let myself = has("myself");
        if has("fail") || has("fail?") || has("handshake") || has("noaddr") {
            continue;
        }
        if !myself && f[7] != "connected" {
            continue;
        }
        if has("master") {
            primaries += 1;
        } else if has("slave") {
            replicas += 1;
        }
    }
    primaries == 0 && replicas > 0
}

async fn server_version(client: &Client) -> Result<Version, ConnectError> {
    let info: String = client
        .info(Some(InfoKind::Server))
        .await
        .map_err(|e| ConnectError::Unreachable(describe(&e)))?;
    info.lines()
        .find_map(|l| l.strip_prefix("redis_version:"))
        .and_then(Version::parse)
        .ok_or_else(|| {
            ConnectError::UnknownVersion(info.lines().take(3).collect::<Vec<_>>().join("; "))
        })
}

/// Ask the server whether it will track for us, by trying.
///
/// A version check is not a substitute: managed platforms disable `CLIENT`
/// subcommands on their own schedule, so the only reliable question is the one
/// the server answers (ADR-0007).
pub async fn probe_tracking(client: &Client) -> bool {
    // OPTIN, no prefixes, no broadcast: tracking applies only to reads we
    // explicitly arm, which is what scopes it to one key rather than to every
    // key a keyspace browse happens to touch.
    client
        .start_tracking(Vec::<String>::new(), false, true, false, false)
        .await
        .is_ok()
}

/// Exponential backoff with a ceiling, so a long outage does not turn into a
/// long silence. The countdown is shown; a silent wait is a freeze wearing a
/// different name (ADR-0009).
pub fn backoff_for(attempt: u32) -> Duration {
    let ms = 250u64.saturating_mul(1 << attempt.min(6));
    Duration::from_millis(ms.min(8_000))
}

/// Whether a connect failure was the server refusing `HELLO`.
fn rejected_hello(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("unknown command") && lower.contains("hello")
}

pub(crate) fn describe(e: &Error) -> String {
    match e.details().is_empty() {
        true => format!("{e}"),
        false => format!("{:?}: {}", e.kind(), e.details()),
    }
}

/// Fetch type, TTL and memory usage for a window of keys (R2.4, PLAN M1.4).
///
/// Three commands per key, pipelined so the whole window costs one round trip
/// rather than `3 × window`. Only ever the visible rows: fetching metadata for
/// a whole keyspace would be `KEYS *` with extra steps, and it would compete
/// with `SCAN` for the connection while the list is still filling.
///
/// A key that vanished between the scan and this fetch is reported as gone
/// rather than failing the batch — the keyspace moves while we walk it. That
/// second return value is free deletion detection: `TYPE` is already on the
/// wire for every visible row, so noticing costs no extra round trip and needs
/// no tracking table (DESIGN §9).
pub async fn fetch_metadata(
    client: &Client,
    keys: &[(usize, Vec<u8>)],
) -> Result<(Vec<MetadataEntry>, Vec<usize>), Error> {
    use redis_pane_core::state::KeyKind;

    let pipeline = client.pipeline();
    for (_, name) in keys {
        let key: Key = name.as_slice().into();
        let _: () = pipeline.r#type(key.clone()).await?;
        let _: () = pipeline.ttl(key.clone()).await?;
        let _: () = pipeline.memory_usage(key, None).await?;
    }
    let replies: Vec<Value> = pipeline.all().await?;

    let mut out = Vec::with_capacity(keys.len());
    let mut gone = Vec::new();
    for (slot, (index, _)) in keys.iter().enumerate() {
        let kind = replies.get(slot * 3).and_then(|v| v.as_str());
        let ttl = replies.get(slot * 3 + 1).and_then(|v| v.as_i64());
        let size = replies.get(slot * 3 + 2).and_then(|v| v.as_i64());

        // TYPE answers "none" for a key that no longer exists. A reply that is
        // absent entirely is a different thing — a short pipeline, not a
        // deleted key — and must not badge the row: one truncated reply would
        // otherwise mark every remaining row in the window as deleted.
        let Some(kind) = kind else { continue };
        if kind == "none" {
            gone.push(*index);
            continue;
        }
        out.push(MetadataEntry {
            index: *index,
            kind: KeyKind::from_redis(&kind),
            // Redis returns -1 for no expiry and -2 for a missing key; both map
            // to "no expiry" here, and the missing case was filtered above.
            ttl_seconds: ttl.unwrap_or(-1).max(-1) as i32,
            size_bytes: size.unwrap_or(0).clamp(0, u32::MAX as i64 - 1) as u32,
        });
    }
    Ok((out, gone))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, flags: &str, link: &str) -> String {
        format!("{id} 127.0.0.1:7000@17000 {flags} - 0 0 1 {link}")
    }

    #[test]
    fn a_cluster_with_a_primary_is_not_replica_only_even_if_the_seed_is_a_replica() {
        let nodes = [
            row("a", "myself,slave", "connected"),
            row("b", "master", "connected"),
        ]
        .join("\n");
        assert!(!cluster_is_all_replica(&nodes));
    }

    #[test]
    fn a_cluster_of_only_replicas_is_replica_only() {
        let nodes = [
            row("a", "myself,slave", "connected"),
            row("b", "slave", "connected"),
        ]
        .join("\n");
        assert!(cluster_is_all_replica(&nodes));
    }

    #[test]
    fn an_unreachable_primary_does_not_count() {
        for (flags, link) in [
            ("master,fail", "connected"),
            ("master,fail?", "connected"),
            ("master,handshake", "connected"),
            ("master,noaddr", "connected"),
            ("master", "disconnected"),
        ] {
            let nodes = [row("a", "myself,slave", "connected"), row("b", flags, link)].join("\n");
            assert!(cluster_is_all_replica(&nodes), "{flags} {link}");
        }
    }

    #[test]
    fn an_empty_or_unparseable_reply_is_not_replica_only() {
        assert!(!cluster_is_all_replica(""));
        assert!(!cluster_is_all_replica("garbage"));
        let only_dead = row("a", "master,fail", "disconnected");
        assert!(!cluster_is_all_replica(&only_dead));
    }

    #[test]
    fn a_normal_cluster_is_not_replica_only() {
        let nodes = [
            row("a", "myself,master", "connected"),
            row("b", "slave", "connected"),
            row("c", "master", "connected"),
        ]
        .join("\n");
        assert!(!cluster_is_all_replica(&nodes));
    }

    #[test]
    fn backoff_grows_then_settles_at_a_ceiling() {
        assert_eq!(backoff_for(0), Duration::from_millis(250));
        assert_eq!(backoff_for(1), Duration::from_millis(500));
        assert_eq!(backoff_for(3), Duration::from_millis(2_000));
        assert_eq!(backoff_for(20), Duration::from_millis(8_000));
    }

    /// A Cluster-scheme URL builds a clustered `Config` (ADR-0022); every
    /// scheme `fred` recognises as clustered is covered because this checks
    /// `config.server.is_clustered()`, not the scheme strings.
    #[test]
    fn a_clustered_url_builds_a_clustered_config() {
        for url in [
            "redis-cluster://127.0.0.1:30001",
            "rediss-cluster://127.0.0.1:30001",
        ] {
            let config = build_config(url, &Credentials::default())
                .expect("a Cluster scheme must build a Config");
            assert!(config.server.is_clustered(), "{url}");
        }
    }

    #[test]
    fn a_plain_scheme_url_is_not_clustered() {
        let config = build_config("redis://127.0.0.1:6379", &Credentials::default())
            .expect("a plain redis:// URL must build a Config");
        assert!(!config.server.is_clustered());
    }

    #[test]
    fn a_rejected_hello_is_recognised_as_an_old_server() {
        assert!(rejected_hello(
            "Unknown: ERR unknown command `HELLO`, with args beginning with: `3`"
        ));
        assert!(!rejected_hello("IO: Connection refused"));
        assert!(!rejected_hello(
            "Auth: WRONGPASS invalid username-password pair"
        ));
    }

    #[test]
    fn the_no_resp3_message_explains_rather_than_leaking_the_protocol_error() {
        let msg = ConnectError::NoResp3 {
            detail: "ERR unknown command `HELLO`".into(),
        }
        .to_string();
        assert!(msg.contains("predates Redis 6.0.0"), "{msg}");
        assert!(msg.contains("RESP3"), "{msg}");
    }

    #[test]
    fn the_below_floor_message_names_both_versions() {
        let msg = ConnectError::BelowFloor {
            found: Version::parse("5.0.14").unwrap(),
        }
        .to_string();
        assert!(msg.contains("5.0.14"), "{msg}");
        assert!(msg.contains("6.0.0"), "{msg}");
    }
}
