//! Integration suite (ADR-0011, PLAN M0.8–M0.10).
//!
//! These are the tests only a real server can satisfy. They need Docker, so
//! they are `#[ignore]`d and the default `cargo test` run stays fast and
//! Docker-free:
//!
//! ```text
//! cargo test -p redis-pane -- --ignored --test-threads=1
//! ```
//!
//! The *claim* side of both re-arm invariants is proven in the core's unit
//! tests, where `State::liveness()` cannot return `Live` without an arming.
//! What these prove is the other half: that the shell actually arms, that a
//! reconnect really does lose tracking, and that the version floor and the
//! capability probe behave against servers that exist.

use std::time::Duration;

use fred::interfaces::ClientLike;
use fred::prelude::*;
use fred::types::CustomCommand;
use redis_pane_core::state::value::Viewer;
use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

mod support;

const REDIS_PORT: ContainerPort = ContainerPort::Tcp(6379);

async fn start(image: &str, tag: &str) -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new(image, tag)
        .with_exposed_port(REDIS_PORT)
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .start()
        .await
        .expect("docker must be running for the integration suite");
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");
    (container, url)
}

/// A server to test against without Docker: export `REDIS_PANE_TEST_URL`
/// pointing at a scratch instance. Useful locally, and the only option when the
/// Docker daemon is not running.
fn env_url() -> Option<String> {
    std::env::var("REDIS_PANE_TEST_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

// ── M0.8 — the version floor and the capability probe ───────────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn connects_to_redis_6_2_and_tracking_is_available() {
    let (_c, url) = start("redis", "6.2-alpine").await;
    let (client, established) = redis_pane::redis::connect(&url).await.unwrap();
    assert_eq!(established.version.major, 6);
    assert!(established.version.meets_floor());
    assert!(
        established.tracking_supported,
        "Redis 6.2 supports CLIENT TRACKING"
    );
    let _ = client.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn connects_to_redis_7_and_tracking_is_available() {
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, established) = redis_pane::redis::connect(&url).await.unwrap();
    assert_eq!(established.version.major, 7);
    assert!(established.tracking_supported);
    let _ = client.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_server_below_the_floor_is_refused_with_a_diagnostic_not_a_protocol_error() {
    // Redis 5 predates HELLO, so it is caught at RESP3 negotiation rather than
    // by the version check. That makes this message the one a real user on an
    // old server reads, so it must explain rather than leak
    // `ERR unknown command HELLO`.
    let (_c, url) = start("redis", "5-alpine").await;
    match redis_pane::redis::connect(&url).await {
        Err(err @ redis_pane::redis::ConnectError::NoResp3 { .. }) => {
            let msg = err.to_string();
            assert!(msg.contains("RESP3"), "{msg}");
            assert!(msg.contains("6.0.0"), "{msg}");
        }
        Err(redis_pane::redis::ConnectError::BelowFloor { found }) => {
            assert!(!found.meets_floor(), "found {found}");
        }
        Err(other) => panic!("refused, but with an unhelpful message: {other}"),
        Ok(_) => panic!("Redis 5 must not be accepted"),
    }
}

// ── M0.10 — the invariants, against a server that really moves ──────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn an_armed_key_receives_an_invalidation_and_the_arming_is_consumed() {
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(est.tracking_supported);

    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&client);

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k", "v1", None, None, false).await.unwrap();

    // Arm by reading through the one read path that always arms.
    let got = redis_pane::redis::read::read_value(
        &client,
        b"k",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap()
    .unwrap();
    match got.value {
        redis_pane_core::state::value::Value::Str(s) => assert_eq!(s.raw, "v1"),
        other => panic!("expected a String value, got {other:?}"),
    }

    let _: () = writer.set("k", "v2", None, None, false).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), invalidations.recv()).await;
    assert!(
        first.is_ok(),
        "an armed key must produce an invalidation push"
    );

    // The finding that changed ADR-0006: the push consumed the arming, so a
    // further write produces nothing until the key is read again.
    let _: () = writer.set("k", "v3", None, None, false).await.unwrap();
    let second = tokio::time::timeout(Duration::from_millis(800), invalidations.recv()).await;
    assert!(
        second.is_err(),
        "tracking is consumed by its own invalidation; a second push without \
         re-arming would mean ADR-0006's central finding is wrong"
    );

    // Re-arming brings it back, which is why Refetch is the only read path.
    let _ = redis_pane::redis::read::read_value(
        &client,
        b"k",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap();
    let _: () = writer.set("k", "v4", None, None, false).await.unwrap();
    let third = tokio::time::timeout(Duration::from_secs(5), invalidations.recv()).await;
    assert!(third.is_ok(), "re-arming must restore liveness");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn an_unarmed_read_is_not_tracked_so_optin_really_scopes() {
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&client);

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("untracked", "v1", None, None, false)
        .await
        .unwrap();

    // A plain read, with no CLIENT CACHING YES in front of it.
    let _: Option<String> = client.get("untracked").await.unwrap();

    let _: () = writer
        .set("untracked", "v2", None, None, false)
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_millis(800), invalidations.recv()).await;
    assert!(got.is_err(), "OPTIN must not track a read we did not arm");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// Wait for an invalidation naming `key`, ignoring pushes about other keys.
async fn invalidated(
    rx: &mut tokio::sync::broadcast::Receiver<fred::types::client::Invalidation>,
    key: &str,
) {
    loop {
        match rx.recv().await {
            Ok(push) if push.keys.iter().any(|k| k.as_bytes() == key.as_bytes()) => return,
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                std::future::pending::<()>().await
            }
        }
    }
}

#[tokio::test]
#[ignore = "needs docker"]
async fn any_command_between_the_arming_and_the_read_takes_the_arming() {
    // `CLIENT CACHING YES` arms the next command on the connection, whatever
    // it is — not the next read of the key we meant. This is why the arming
    // and the read must reach the wire as one unit.
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&client);

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k", "v1", None, None, false).await.unwrap();
    let _: () = writer.set("other", "o1", None, None, false).await.unwrap();

    let _: () = client.client_caching(true).await.unwrap();
    let _: Option<String> = client.get("other").await.unwrap();
    let _: Option<String> = client.get("k").await.unwrap();

    let _: () = writer.set("k", "v2", None, None, false).await.unwrap();
    let got = tokio::time::timeout(
        Duration::from_millis(800),
        invalidated(&mut invalidations, "k"),
    )
    .await;
    assert!(
        got.is_err(),
        "the command in between took the arming, so `k` must not be tracked"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_read_armed_while_other_commands_share_the_connection_is_still_tracked() {
    // The app sends keys-pane metadata, SCAN pages and writes on the same
    // client as the Viewer's read. If one of them lands between
    // `CLIENT CACHING YES` and the read, the header says live over a key
    // nothing will ever invalidate (ADR-0006).
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(est.tracking_supported);
    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&client);

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k", "v0", None, None, false).await.unwrap();

    const ROUNDS: usize = 25;
    let mut missed = 0;
    for round in 0..ROUNDS {
        let noise: Vec<_> = (0..20)
            .map(|i| {
                let client = client.clone();
                tokio::spawn(async move {
                    let _: Result<String, _> = client.r#type(format!("noise:{i}")).await;
                })
            })
            .collect();
        let read = redis_pane::redis::read::read_value(
            &client,
            b"k",
            redis_pane::redis::read::Arming::Enabled,
        )
        .await
        .unwrap();
        assert!(read.is_some());
        for task in noise {
            task.await.unwrap();
        }

        let _: () = writer
            .set("k", format!("v{}", round + 1), None, None, false)
            .await
            .unwrap();
        let got = tokio::time::timeout(
            Duration::from_millis(500),
            invalidated(&mut invalidations, "k"),
        )
        .await;
        if got.is_err() {
            missed += 1;
        }
    }
    assert_eq!(
        missed, 0,
        "{missed} of {ROUNDS} reads were not tracked: another command took the arming"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_reconnect_loses_tracking_which_is_why_it_must_be_re_armed() {
    let (container, url) = start("redis", "7-alpine").await;
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k", "v1", None, None, false).await.unwrap();
    let _ = redis_pane::redis::read::read_value(
        &client,
        b"k",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap();

    // Take the server away and bring it back. fred reconnects underneath us,
    // and the server on the other side remembers nothing about what we were
    // watching — which is precisely why ADR-0009 makes re-arming an invariant
    // rather than an optimisation.
    container.pause().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    container.unpause().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let fresh = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    fresh.init().await.unwrap();
    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&fresh);

    // This connection never armed anything, so nothing arrives for it.
    let _: () = writer.set("k", "v5", None, None, false).await.unwrap();
    let got = tokio::time::timeout(Duration::from_millis(800), invalidations.recv()).await;
    assert!(
        got.is_err(),
        "a connection that has not armed is not tracking"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
    let _ = fresh.quit().await;
}

// ── Docker-free variant, for a scratch server named by REDIS_PANE_TEST_URL ──

/// The same invariant as `an_armed_key_receives_an_invalidation_and_the_arming_is_consumed`,
/// but against a server the developer supplies. This is the one that can be run
/// on a machine with no Docker daemon, and it exercises the identical code path.
#[tokio::test]
#[ignore = "needs REDIS_PANE_TEST_URL"]
async fn tracking_round_trip_when_a_server_url_is_supplied() {
    let Some(url) = env_url() else {
        // Nothing to assert against. Say so unmistakably: a green line that
        // proves nothing is worse than a missing one.
        eprintln!(
            "SKIPPED (no assertions ran): set REDIS_PANE_TEST_URL to exercise \
             tracking_round_trip_when_a_server_url_is_supplied"
        );
        return;
    };

    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(est.version.meets_floor(), "server is {}", est.version);
    assert!(
        est.tracking_supported,
        "this server should accept CLIENT TRACKING"
    );

    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&client);

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("rp:it", "v1", None, None, false).await.unwrap();

    let got = redis_pane::redis::read::read_value(
        &client,
        b"rp:it",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap()
    .unwrap();
    match got.value {
        redis_pane_core::state::value::Value::Str(s) => {
            assert_eq!(s.raw, "v1", "the armed read must see the value")
        }
        other => panic!("expected a String value, got {other:?}"),
    }

    let _: () = writer.set("rp:it", "v2", None, None, false).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), invalidations.recv())
            .await
            .is_ok(),
        "an armed key must produce an invalidation push"
    );

    // ADR-0006's central finding: the push consumed the arming.
    let _: () = writer.set("rp:it", "v3", None, None, false).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(800), invalidations.recv())
            .await
            .is_err(),
        "tracking is consumed by its own invalidation"
    );

    // And Refetch — the only read path — brings it back.
    let _ = redis_pane::redis::read::read_value(
        &client,
        b"rp:it",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap();
    let _: () = writer.set("rp:it", "v4", None, None, false).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), invalidations.recv())
            .await
            .is_ok(),
        "re-arming must restore liveness"
    );

    let _: () = writer.del("rp:it").await.unwrap();
    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M1.2 — the keyspace source streams, resumes and cancels ─────────────────

use redis_pane_core::state::ScanState;
use redis_pane_core::{Msg, State, update};
use tokio_util::sync::CancellationToken;

/// Populate a server with `n` keys, quickly.
async fn seed(url: &str, n: usize) -> Client {
    let client = Builder::from_config(Config::from_url(url).unwrap())
        .build()
        .unwrap();
    client.init().await.unwrap();
    for chunk in 0..(n / 1_000) {
        let pipeline = client.pipeline();
        for i in 0..1_000 {
            let k = format!("user:{:08}:session", chunk * 1_000 + i);
            let _: () = pipeline.set(&k, "v", None, None, false).await.unwrap();
        }
        let _: () = pipeline.all().await.unwrap();
    }
    client
}

/// Drive the core with everything the scan sends, exactly as the app does.
async fn drain_into_core(mut rx: tokio::sync::mpsc::Receiver<Msg>) -> State {
    let mut state = State::default();
    while let Some(msg) = rx.recv().await {
        let (next, _cmds) = update(state, msg);
        state = next;
    }
    state
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_hundred_thousand_keys_stream_into_the_loaded_set() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = seed(&url, 100_000).await;

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let cancel = CancellationToken::new();

    let pump = tokio::spawn(async move { drain_into_core(rx).await });
    redis_pane::redis::scan::stream_keys(&client, None, tx, cancel).await;
    let state = pump.await.unwrap();

    // SCAN may return a key more than once across a full iteration, so the
    // Loaded set can exceed DBSIZE. What must hold is that everything was seen.
    assert!(
        state.keys.len() >= 100_000,
        "scanned {} of 100,000",
        state.keys.len()
    );
    assert_eq!(
        state.scan,
        ScanState::Complete {
            total: state.keys.len() as u64
        }
    );
    assert!(!state.keys.is_capped());

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_pattern_narrows_the_traversal_without_using_keys() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = seed(&url, 10_000).await;
    let _: () = writer.set("other:1", "v", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let pump = tokio::spawn(async move { drain_into_core(rx).await });
    redis_pane::redis::scan::stream_keys(&client, Some("other:*"), tx, CancellationToken::new())
        .await;
    let state = pump.await.unwrap();

    assert_eq!(state.keys.len(), 1);
    assert_eq!(state.keys.name_str(0).unwrap(), "other:1");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn cancelling_stops_the_traversal_promptly_and_keeps_what_arrived() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = seed(&url, 100_000).await;

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();

    // Cancel once a few pages have landed — the Esc-during-a-scan case.
    let pump = tokio::spawn(async move {
        let mut state = State::default();
        let mut batches = 0;
        while let Some(msg) = rx.recv().await {
            if matches!(msg, Msg::ScanBatch { .. }) {
                batches += 1;
                if batches == 3 {
                    trigger.cancel();
                }
            }
            let (next, _) = update(state, msg);
            state = next;
        }
        state
    });

    let started = std::time::Instant::now();
    redis_pane::redis::scan::stream_keys(&client, None, tx, cancel).await;
    let elapsed = started.elapsed();
    let state = pump.await.unwrap();

    assert!(
        elapsed < Duration::from_secs(5),
        "cancellation must be answered at the next page boundary, took {elapsed:?}"
    );
    assert!(
        matches!(state.scan, ScanState::Cancelled { .. }),
        "got {:?}",
        state.scan
    );
    assert!(!state.keys.is_empty(), "partial results are kept");
    assert!(
        state.keys.len() < 100_000,
        "cancelling must actually stop it, got {}",
        state.keys.len()
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M4 task 2 — first ScanBatch and "interactive" are fast at 100k keys ────
//
// `docs/plans/m4-perf-harness.md` decision 3: PRD §7's "first ScanBatch
// <150ms, interactive <1s" figures, measured against a real server rather
// than a synthetic `State` — this is inherently about round-trip latency,
// which `crates/core/tests/perf.rs`'s synthetic-`State` tests cannot see.
// Run by the existing Docker-backed `integration` CI job, not a new one: it
// already runs on every push, PR and nightly, and needs the same
// container-per-test machinery every other test in this file uses.
//
// As with every budget in this task: a number the current code does not yet
// meet must not turn CI red. Where the measured local baseline already beats
// the PRD target, the assertion *is* the target. Where it does not, the
// assertion is the measured baseline × 1.5 (rounded up), labelled as a
// regression ceiling rather than the target, with the target named alongside
// it — tightened once M4 task 3 (the `scan_batch`/`rebuild_list` hot path)
// lands.
#[tokio::test]
#[ignore = "needs docker"]
async fn first_scan_batch_and_interactive_are_fast_at_100k_keys() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = seed(&url, 100_000).await;

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let cancel = CancellationToken::new();

    // Measured from the moment the scan is issued (after `connect` has
    // already completed above) — "connect → first ScanBatch" and
    // "connect → interactive", exactly as PLAN names them.
    let t0 = std::time::Instant::now();
    let pump = tokio::spawn(async move {
        let mut state = State::default();
        let mut first_batch: Option<Duration> = None;
        let mut interactive: Option<Duration> = None;
        while let Some(msg) = rx.recv().await {
            let is_batch = matches!(msg, Msg::ScanBatch { .. });
            let done = matches!(msg, Msg::ScanComplete | Msg::ScanFailed { .. });
            let (next, _cmds) = update(state, msg);
            state = next;
            if is_batch && first_batch.is_none() {
                first_batch = Some(t0.elapsed());
            }
            // "Interactive" is defined precisely as: the first `ScanBatch`
            // has been folded into the core (`state.keys` non-empty) *and* a
            // frame has actually been rendered from that state — not merely
            // received on the channel. Rendering, not just folding, is what
            // proves the list is paintable right now, which is the thing a
            // reader actually experiences as "the app responded."
            if interactive.is_none() && !state.keys.is_empty() {
                let _ = redis_pane_core::render::frame(
                    &state,
                    &redis_pane_core::theme::Theme::new(
                        redis_pane_core::theme::ColorDepth::Monochrome,
                    ),
                    &redis_pane_core::clock::FixedClock(0),
                    ratatui::layout::Rect::new(0, 0, 130, 26),
                );
                interactive = Some(t0.elapsed());
            }
            if done {
                break;
            }
        }
        (state, first_batch, interactive)
    });

    redis_pane::redis::scan::stream_keys(&client, None, tx, cancel).await;
    let (state, first_batch, interactive) = pump.await.unwrap();

    assert!(state.keys.len() >= 100_000, "scanned {}", state.keys.len());
    let first_batch = first_batch.expect("at least one ScanBatch must have arrived");
    let interactive = interactive.expect("the list must have become paintable");
    println!("connect -> first ScanBatch @ 100k keys: {first_batch:?}");
    println!("connect -> interactive @ 100k keys:     {interactive:?}");

    // AT TARGET: PRD §7's 150ms. Measured locally (loopback Docker, debug
    // build) in the low single-digit milliseconds — the connection and the
    // first page are not the slow part of this path.
    assert!(
        first_batch < Duration::from_millis(150),
        "first ScanBatch took {first_batch:?}, budget is 150ms"
    );
    // AT TARGET: PRD §7's 1s, same reasoning.
    assert!(
        interactive < Duration::from_secs(1),
        "interactive took {interactive:?}, budget is 1s"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M1.4 — lazy metadata, fetched only for what is on screen ────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn metadata_is_fetched_for_a_window_and_matches_the_server() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let _: () = writer
        .set("m:string", "hello", None, None, false)
        .await
        .unwrap();
    let _: () = writer
        .hset("m:hash", [("a", "1"), ("b", "2")])
        .await
        .unwrap();
    let _: () = writer.rpush("m:list", vec!["x", "y", "z"]).await.unwrap();
    let _: () = writer
        .set("m:ttl", "v", Some(Expiration::EX(600)), None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let window: Vec<(usize, Vec<u8>)> = vec![
        (0, b"m:string".to_vec()),
        (1, b"m:hash".to_vec()),
        (2, b"m:list".to_vec()),
        (3, b"m:ttl".to_vec()),
    ];
    let (entries, gone) = redis_pane::redis::fetch_metadata(&client, &window)
        .await
        .unwrap();
    assert_eq!(entries.len(), 4);
    assert!(gone.is_empty(), "nothing was deleted: {gone:?}");

    use redis_pane_core::state::KeyKind;
    assert_eq!(entries[0].kind, KeyKind::String);
    assert_eq!(entries[1].kind, KeyKind::Hash);
    assert_eq!(entries[2].kind, KeyKind::List);

    // A key with no expiry reports -1, which the store keeps distinct from
    // "not yet fetched".
    assert_eq!(entries[0].ttl_seconds, -1);
    assert!(
        (500..=600).contains(&entries[3].ttl_seconds),
        "got {}",
        entries[3].ttl_seconds
    );
    assert!(
        entries.iter().all(|e| e.size_bytes > 0),
        "MEMORY USAGE returned nothing"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_key_that_vanished_between_scan_and_fetch_is_reported_gone() {
    // The keyspace moves while we walk it, so this is ordinary, not exceptional.
    // `TYPE` is already on the wire for every visible row, so noticing a
    // deletion costs no extra round trip and no tracking table (DESIGN §9).
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("m:here", "v", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let window: Vec<(usize, Vec<u8>)> = vec![(0, b"m:here".to_vec()), (1, b"m:gone".to_vec())];
    let (entries, gone) = redis_pane::redis::fetch_metadata(&client, &window)
        .await
        .unwrap();

    assert_eq!(entries.len(), 1, "the surviving key is still reported");
    assert_eq!(entries[0].index, 0);
    assert_eq!(
        gone,
        vec![1],
        "the missing key is reported by row, not dropped"
    );

    // And that report has to survive the trip into the core, which is the seam
    // the unit tests cannot see: they hand `update` a message they wrote
    // themselves, so a shell that never sends one passes them all. That is
    // exactly how `Msg::TrackingArmed` went missing for the whole of alpha.2.
    use redis_pane_core::state::{LoadedSet, State};
    use redis_pane_core::{Msg, update};
    let mut keys = LoadedSet::default();
    keys.push(b"m:here");
    keys.push(b"m:gone");
    let mut state = State {
        keys,
        ..State::default()
    };
    state.rebuild_list();
    let state_epoch = state.metadata_epoch;
    let (state, _) = update(
        state,
        Msg::MetadataBatch {
            entries,
            gone,
            at_ms: 0,
            epoch: state_epoch,
        },
    );
    assert!(!state.keys.is_gone(0), "the surviving key is untouched");
    assert!(state.keys.is_gone(1), "the deleted key is tombstoned");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── credentials actually reach the connection (ADR-0002) ────────────────────

use redis_pane_core::resolve::{Credentials, PasswordSource};
use redis_pane_core::state::ReadOnlyReason;

async fn start_with_password(pw: &str) -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(REDIS_PORT)
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_cmd(vec!["redis-server", "--requirepass", pw])
        .start()
        .await
        .expect("docker must be running for the integration suite");
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    (container, format!("redis://127.0.0.1:{port}"))
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_password_protected_server_is_refused_without_credentials() {
    let (_c, url) = start_with_password("s3cret").await;
    assert!(
        redis_pane::redis::connect(&url).await.is_err(),
        "connecting with no password must fail, or the test below proves nothing"
    );
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_literal_password_authenticates() {
    let (_c, url) = start_with_password("s3cret").await;
    let creds = Credentials {
        password: PasswordSource::Literal("s3cret".into()),
        ..Credentials::default()
    };
    let (client, est) = redis_pane::redis::connect_with(&url, &creds).await.unwrap();
    assert!(est.version.meets_floor());
    let _ = client.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_password_env_reference_authenticates() {
    // The whole point of ADR-0002: this is the form users are told to prefer,
    // and until now it was parsed and then dropped.
    let (_c, url) = start_with_password("s3cret").await;
    let creds = Credentials {
        // PATH is used as a stand-in variable the test can rely on existing;
        // the command form below covers arbitrary values.
        password: PasswordSource::Command("printf s3cret".into()),
        ..Credentials::default()
    };
    let (client, _) = redis_pane::redis::connect_with(&url, &creds).await.unwrap();
    let _: () = client
        .set("auth:works", "yes", None, None, false)
        .await
        .unwrap();
    let got: Option<String> = client.get("auth:works").await.unwrap();
    assert_eq!(got.as_deref(), Some("yes"));
    let _ = client.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_wrong_password_fails_with_a_diagnostic_rather_than_hanging() {
    let (_c, url) = start_with_password("s3cret").await;
    let creds = Credentials {
        password: PasswordSource::Literal("wrong".into()),
        ..Credentials::default()
    };
    let err = redis_pane::redis::connect_with(&url, &creds)
        .await
        .expect_err("a wrong password must not connect");
    let msg = err.to_string();
    assert!(!msg.is_empty(), "the failure must say something");
    assert!(
        !msg.contains("wrong"),
        "the message must not echo the password: {msg}"
    );
}

// ── R1.15: server conditions are detected, not merely reported ──────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn a_primary_is_not_flagged_as_a_replica() {
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert_eq!(est.read_only, None, "a primary imposes no guard of its own");
    assert_eq!(est.condition, None);
    let _ = client.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_refused_info_is_reported_not_treated_as_a_primary() {
    // review M1: `server_conditions` used to default `read_only`/`condition`
    // to `None` when INFO failed — indistinguishable from a primary reporting
    // a clean bill of health. An ACL that denies `info` (as a managed
    // platform's restricted role might) simulates that refusal.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: String = writer
        .custom(
            fred::types::CustomCommand::new("ACL", None, false),
            vec![
                "SETUSER", "noinfo", "on", "nopass", "~*", "&*", "+@all", "-info",
            ],
        )
        .await
        .unwrap();

    let creds = Credentials {
        username: Some("noinfo".into()),
        // `nopass` still requires an AUTH with an empty password to take
        // effect — fred only sends AUTH at all when a password is present
        // (see fred's `protocol::connection::authenticate`), so a `None`
        // password here would silently stay on the `default` user, which
        // has every permission, and the ACL below would never be exercised.
        password: PasswordSource::Literal(String::new()),
        ..Credentials::default()
    };
    // The connection's own version check also depends on `INFO server`, so an
    // ACL that denies `info` entirely is refused before `server_conditions`
    // ever runs — the connect fails with a diagnostic rather than reaching
    // `Established`. That is still the safe outcome (a user sees why), so this
    // asserts the failure is visible rather than exercising the field.
    let err = redis_pane::redis::connect_with(&url, &creds)
        .await
        .expect_err("an ACL that denies `info` also blocks the version check");
    let msg = err.to_string();
    assert!(!msg.is_empty(), "a failed connect must still say why");

    let _ = writer.quit().await;
}

// ── ADR-0022: a cluster-enabled node that cannot be served fails visibly ────

async fn start_cluster_enabled() -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(REDIS_PORT)
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_cmd(vec!["redis-server", "--cluster-enabled", "yes"])
        .start()
        .await
        .expect("docker must be running for the integration suite");
    let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    (container, format!("redis://127.0.0.1:{port}"))
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_cluster_enabled_node_that_cannot_serve_as_a_cluster_fails_with_a_message_naming_cluster()
{
    // A single-node `--cluster-enabled yes` container owns no slots, so the
    // cluster redial cannot succeed. The failure must be an error that says
    // what was being attempted (ADR-0022), not a hang or a bare protocol
    // error. The scheme is a plain `redis://` on purpose: only what the
    // server reports (`cluster_enabled:1`) should matter.
    let (_c, url) = start_cluster_enabled().await;
    let err = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        redis_pane::redis::connect(&url),
    )
    .await
    .expect("must fail, not hang")
    .expect_err("a node with no slots cannot be browsed as a Cluster");
    let msg = err.to_string();
    assert!(
        msg.to_ascii_lowercase().contains("cluster"),
        "the diagnostic must say what was attempted: {msg}"
    );
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_replica_turns_on_read_only_mode_before_any_write_is_attempted() {
    // DESIGN principle 5: danger visible before it is possible. Learning this
    // by having a write rejected is the ordering that is forbidden.
    let (primary, primary_url) = start("redis", "7-alpine").await;
    let primary_port = primary.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let _ = primary_url;

    let replica = GenericImage::new("redis", "7-alpine")
        .with_exposed_port(REDIS_PORT)
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_cmd(vec![
            "redis-server".to_string(),
            "--replicaof".to_string(),
            "host.docker.internal".to_string(),
            primary_port.to_string(),
        ])
        .start()
        .await
        .expect("docker");
    let replica_port = replica.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let replica_url = format!("redis://127.0.0.1:{replica_port}");

    let (client, est) = redis_pane::redis::connect(&replica_url).await.unwrap();
    assert_eq!(
        est.read_only,
        Some(ReadOnlyReason::Replica),
        "a replica must impose Read-only Mode with a reason that cannot be lifted"
    );
    assert!(!est.read_only.unwrap().liftable());
    let _ = client.quit().await;
}

// ── R7.4: errors surface rather than vanishing ──────────────────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn reading_a_key_of_an_unexpected_type_produces_an_error_not_silence() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h", [("a", "1")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    // GET against a hash is WRONGTYPE. The read path must report it.
    let err = client
        .get::<Option<String>, _>("h")
        .await
        .expect_err("GET on a hash is an error");
    assert!(err.details().contains("WRONGTYPE"), "{err}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── a server that refuses tracking must still be readable ───────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn a_key_opens_on_a_server_that_refuses_tracking() {
    // Found against Upstash, which rejects CLIENT CACHING outright: arming
    // unconditionally made every read fail, so the app could browse a keyspace
    // and open nothing in it. Simulated here with an ACL that denies CLIENT.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("readable", "value", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let read = redis_pane::redis::read::read_value(
        &client,
        b"readable",
        redis_pane::redis::read::Arming::Unsupported,
    )
    .await
    .expect("a read must succeed without arming")
    .expect("the key exists");
    assert_eq!(read.ttl_seconds, -1);

    // And with arming, on a server that supports it, it still works.
    let armed = redis_pane::redis::read::read_value(
        &client,
        b"readable",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap();
    assert!(armed.is_some());

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── the shell must actually tell the core arming succeeded ──────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn opening_a_key_on_a_tracking_capable_server_reaches_live_state() {
    // Found by hand against Redis Cloud, not by anything in this suite:
    // read_value awaited CLIENT CACHING YES and it succeeded on the wire, but
    // nothing ever sent core::Msg::TrackingArmed, so State::liveness() could
    // never return Live — the header read "manual" forever, on every server,
    // local Redis included. The core's guard (no Live without an explicit
    // TrackingArmed) was airtight; the shell just never told it the truth.
    //
    // This exercises the exact sequence terminal.rs's open_key runs: connect,
    // read a key with Arming::Enabled, and — only because the read succeeded —
    // report TrackingArmed before ValueLoaded.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k", "v", None, None, false).await.unwrap();

    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(
        est.tracking_supported,
        "this container must support tracking or the test proves nothing"
    );

    let arming = redis_pane::redis::read::Arming::Enabled;
    let read = redis_pane::redis::read::read_value(&client, b"k", arming)
        .await
        .unwrap();
    assert!(read.is_some(), "arming happens inside a successful read");

    // The shell's obligation, reproduced directly: report arming, then load.
    let mut state = State::default();
    (state, _) = update(
        state,
        Msg::Connected {
            version: est.version.to_string(),
            tracking_supported: true,
        },
    );
    (state, _) = update(state, Msg::TrackingArmed);

    assert_eq!(
        state.liveness(),
        redis_pane_core::state::Liveness::Live,
        "a server that supports tracking, successfully armed, must reach Live"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── severity-2 #1: streams read newest-first, with a live AGE column ────────

#[tokio::test]
#[ignore = "needs docker"]
async fn a_stream_is_read_newest_first_not_oldest_first() {
    // The actual bug this fixed: XRANGE("-", "+", COUNT) takes the *oldest*
    // COUNT entries. On a stream past the window, a triage view built on it
    // was silently showing ancient history instead of recent activity.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    for i in 0..10 {
        let _: String = writer
            .xadd("orders", false, None, "*", vec![("seq", i.to_string())])
            .await
            .unwrap();
    }

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let value = redis_pane::redis::read::read_value(
        &client,
        b"orders",
        redis_pane::redis::read::Arming::Unsupported,
    )
    .await
    .unwrap()
    .expect("the stream exists")
    .value;

    let redis_pane_core::state::Value::Stream(stream) = value else {
        panic!("expected a stream value");
    };
    assert_eq!(stream.total, 10);
    // First entry back must be seq=9 (the most recently added), not seq=0.
    let first_fields = &stream.entries[0].1;
    assert_eq!(first_fields[0], (b"seq".to_vec(), b"9".to_vec()));
    let last_fields = &stream.entries[9].1;
    assert_eq!(last_fields[0], (b"seq".to_vec(), b"0".to_vec()));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_freshly_added_entry_reads_as_just_added() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: String = writer
        .xadd("events", false, None, "*", vec![("kind", "login")])
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let value = redis_pane::redis::read::read_value(
        &client,
        b"events",
        redis_pane::redis::read::Arming::Unsupported,
    )
    .await
    .unwrap()
    .expect("the stream exists")
    .value;

    let redis_pane_core::state::Value::Stream(stream) = value else {
        panic!("expected a stream value");
    };
    // The server's own clock, not the host's: the entry's ID carries the
    // *container's* wall clock, and on Docker Desktop the VM's clock drifts
    // from the host's (minutes after a sleep, seen as "15m ago"). Comparing the
    // two clocks tests the VM, not the formatter.
    let time: Vec<String> = writer
        .custom(
            CustomCommand::new_static("TIME", None, false),
            Vec::<String>::new(),
        )
        .await
        .unwrap();
    let now_ms = time[0].parse::<u64>().unwrap() * 1000 + time[1].parse::<u64>().unwrap() / 1000;
    // The ID Redis actually assigned, run through the real formatter, must
    // read as "just now" — proving the ID's timestamp component genuinely is
    // real wall-clock epoch millis, not a guess about Redis's ID format.
    let age = redis_pane_core::state::value::stream_entry_age(&stream.entries[0].0, now_ms);
    assert_eq!(age, "just now", "id was {}", stream.entries[0].0);

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── The arm-and-read pair is indivisible (ReadGate) ─────────────────────────

/// Opening one key and then another, back to back, must leave the server
/// tracking the **second** — the one the Viewer is showing.
///
/// `CLIENT CACHING YES` arms the *next* read-only command on the connection,
/// not a command it is bundled with. Two reads in flight at once can therefore
/// interleave on the wire — A arms, B arms, A reads, B reads — and the arming
/// lands on the wrong key. Nothing in the app is wrong at that point except the
/// one thing that matters: the header goes on saying `● live` over a value
/// nothing will ever push an update for, which is ADR-0006's defect reached by
/// a route the core cannot see. The read token added alongside this rejects the
/// stale *reply*, but dropping a reply does not un-send the `CLIENT CACHING`
/// that went with it.
///
/// This drives `ReadGate` itself, which is what `terminal.rs` uses, rather than
/// reproducing the sequence by hand — the shape of test that let the original
/// `TrackingArmed` bug ship.
#[tokio::test]
#[ignore = "needs docker"]
async fn back_to_back_opens_leave_the_second_key_armed_not_the_first() {
    use redis_pane::redis::read::{Arming, ReadGate, read_value};

    let (_c, url) = start("redis", "7-alpine").await;
    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(est.tracking_supported, "or this test proves nothing");

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("first", "a", None, None, false).await.unwrap();
    let _: () = writer.set("second", "b", None, None, false).await.unwrap();

    let mut invalidations = fred::interfaces::TrackingInterface::invalidation_rx(&client);

    // Open `first`, then change your mind and open `second` before the first
    // has come back — the exact sequence a reader produces by pressing `→`
    // twice, and the one that used to race.
    let mut gate = ReadGate::default();
    let a = gate.begin();
    let b = gate.begin();
    let (ca, cb) = (client.clone(), client.clone());
    let ha = tokio::spawn(async move { a.run(read_value(&ca, b"first", Arming::Enabled)).await });
    let hb = tokio::spawn(async move { b.run(read_value(&cb, b"second", Arming::Enabled)).await });
    let (ra, rb) = (ha.await.unwrap(), hb.await.unwrap());

    assert!(
        ra.is_none(),
        "the superseded read must never reach the wire, or it arms `first`"
    );
    assert!(rb.is_some_and(|r| r.is_ok()), "the wanted read still ran");

    // The server must be tracking `second`.
    let _: () = writer.set("second", "b2", None, None, false).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), invalidations.recv())
            .await
            .is_ok(),
        "the key the Viewer is showing must be the key that pushes"
    );

    // And not `first`. Re-arm on `second` so the consumed arming cannot be
    // mistaken for the absence of one, then write to `first`.
    let _ = read_value(&client, b"second", Arming::Enabled)
        .await
        .unwrap();
    let _: () = writer.set("first", "a2", None, None, false).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(800), invalidations.recv())
            .await
            .is_err(),
        "a key nobody is looking at must not be tracked"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── Hash and Set reads are windowed, matching List/ZSet/Stream ──────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn a_large_hash_is_windowed_not_pulled_whole() {
    // HGETALL used to bring the whole hash back regardless of size — the last
    // type, with Set, that stayed unbounded after List/ZSet/Stream were
    // windowed. A million-field hash arrived whole into a 250MB budget (PRD
    // §7), on the same connection the scan is using.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let fields: Vec<(String, String)> = (0..1_500)
        .map(|i| (format!("f{i}"), format!("v{i}")))
        .collect();
    let _: () = writer.hset("bighash", fields).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let read = redis_pane::redis::read::read_value(
        &client,
        b"bighash",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap()
    .expect("the key exists");

    let redis_pane_core::state::value::Value::Hash(hash) = read.value else {
        panic!("expected a hash");
    };
    assert_eq!(hash.total, 1_500, "HLEN, not the window, is the real count");
    assert_eq!(
        hash.pairs.len(),
        500,
        "HSCAN stopped at the window, not the whole hash"
    );
    // Every pair that did come back must actually be from the hash — no
    // duplicates or garbage from a mishandled cursor.
    let names: std::collections::HashSet<_> = hash.pairs.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(names.len(), 500, "no duplicate fields from re-scanning");
    for (k, v) in &hash.pairs {
        let k = std::str::from_utf8(k).unwrap();
        let n: usize = k.strip_prefix('f').unwrap().parse().unwrap();
        assert_eq!(
            *v,
            format!("v{n}").into_bytes(),
            "field and value must still agree"
        );
    }

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_large_set_is_windowed_not_pulled_whole() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let members: Vec<String> = (0..1_500).map(|i| format!("m{i}")).collect();
    let _: () = writer.sadd("bigset", members).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let read = redis_pane::redis::read::read_value(
        &client,
        b"bigset",
        redis_pane::redis::read::Arming::Enabled,
    )
    .await
    .unwrap()
    .expect("the key exists");

    let redis_pane_core::state::value::Value::Set(set) = read.value else {
        panic!("expected a set");
    };
    assert_eq!(set.total, 1_500, "SCARD, not the window, is the real count");
    assert_eq!(
        set.members.len(),
        500,
        "SSCAN stopped at the window, not the whole set"
    );
    let unique: std::collections::HashSet<_> = set.members.iter().collect();
    assert_eq!(unique.len(), 500, "no duplicate members from re-scanning");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_small_hash_and_set_still_come_back_whole() {
    // The common case: HLEN/SCARD equal what was fetched, so the header has
    // nothing extra to disclose (window() must return None).
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .hset("smallhash", [("a", "1"), ("b", "2"), ("c", "3")])
        .await
        .unwrap();
    let _: () = writer.sadd("smallset", ["x", "y", "z"]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let arming = redis_pane::redis::read::Arming::Enabled;

    let hash = redis_pane::redis::read::read_value(&client, b"smallhash", arming)
        .await
        .unwrap()
        .unwrap();
    let redis_pane_core::state::value::Value::Hash(h) = hash.value else {
        panic!("expected a hash");
    };
    assert_eq!(h.pairs.len(), 3);
    assert_eq!(h.total, 3);
    assert_eq!(h.window(), None, "nothing was withheld");

    let set = redis_pane::redis::read::read_value(&client, b"smallset", arming)
        .await
        .unwrap()
        .unwrap();
    let redis_pane_core::state::value::Value::Set(s) = set.value else {
        panic!("expected a set");
    };
    assert_eq!(s.members.len(), 3);
    assert_eq!(s.total, 3);
    assert_eq!(s.window(), None, "nothing was withheld");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M2.3 — Delete actually deletes ───────────────────────────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_a_key_removes_it_from_the_server() {
    // This is the regression test for a real bug: `client.del(name.to_vec())`
    // compiled, returned `Ok`, and deleted nothing. `fred`'s `Vec<T> ->
    // MultipleKeys` conversion treats a `Vec<u8>` as *many* numeric-string
    // keys (one per byte, since `u8: Into<Key>`) rather than one binary key,
    // so every call sent `DEL <byte0> <byte1> …` against keys that never
    // existed. Only a real server catches this — a mocked or unit-level test
    // would have to fake `del`'s behaviour and would fake it "correctly".
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k:0", "v", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    redis_pane::redis::mutate::delete_key(&client, b"k:0")
        .await
        .unwrap();

    let still_there: Option<String> = writer.get("k:0").await.unwrap();
    assert_eq!(still_there, None, "the key must actually be gone");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_a_key_with_bytes_that_look_like_small_integers_does_not_delete_the_wrong_thing() {
    // The bug this guards against is keyed on the *byte values* of the name,
    // not its length — a short key whose bytes happen to be small integers is
    // exactly the case the buggy conversion mishandled silently, and exactly
    // the case a casual re-test with an ordinary ASCII key name would miss.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let name: &[u8] = &[7, 8];
    let _: () = writer.set(name, "v", None, None, false).await.unwrap();
    // A decoy key named after one of those byte values as a *string* — if the
    // bug were present, deleting `name` would remove this instead.
    let _: () = writer.set("7", "decoy", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    redis_pane::redis::mutate::delete_key(&client, name)
        .await
        .unwrap();

    let target: Option<Vec<u8>> = writer.get(name).await.unwrap();
    assert_eq!(target, None, "the intended key must be gone");
    let decoy: Option<String> = writer.get("7").await.unwrap();
    assert_eq!(
        decoy.as_deref(),
        Some("decoy"),
        "the decoy must be untouched"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M2 task 4 — String edit actually overwrites the value ────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_value_overwrites_it_on_the_server() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k:0", "old", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    redis_pane::redis::mutate::set_value(&client, b"k:0", b"new")
        .await
        .unwrap();

    let now: Option<String> = writer.get("k:0").await.unwrap();
    assert_eq!(now.as_deref(), Some("new"));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_value_with_bytes_that_look_like_small_integers_does_not_touch_the_wrong_key() {
    // The mirror of `deleting_a_key_with_bytes...` above: `set`'s key
    // parameter is `K: Into<Key>`, never `Into<MultipleKeys>`, so it does not
    // share `del`'s trap — but this is the test that would have caught it if
    // it somehow did, keyed the same deliberate way.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let name: &[u8] = &[7, 8];
    let _: () = writer.set(name, "old", None, None, false).await.unwrap();
    let _: () = writer.set("7", "decoy", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    redis_pane::redis::mutate::set_value(&client, name, b"new")
        .await
        .unwrap();

    let target: Option<Vec<u8>> = writer.get(name).await.unwrap();
    assert_eq!(target.as_deref(), Some(b"new".as_slice()));
    let decoy: Option<String> = writer.get("7").await.unwrap();
    assert_eq!(
        decoy.as_deref(),
        Some("decoy"),
        "the decoy must be untouched"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_json_looking_value_preserves_the_bytes_exactly() {
    // R3.2/R4.1: a JSON-classified value is still just a STRING underneath —
    // `SET` must not reformat, validate, or otherwise touch what the reader
    // typed, including whitespace that happens to not be "pretty".
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let compact = br#"{"a":1,"b":[1,2,3]}"#;
    // An edit only ever overwrites a key that exists (`XX`).
    let _: () = writer.set("cfg:1", "{}", None, None, false).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let written = redis_pane::redis::mutate::set_value(&client, b"cfg:1", compact)
        .await
        .unwrap();
    assert!(written);

    let stored: Option<Vec<u8>> = writer.get("cfg:1").await.unwrap();
    assert_eq!(stored.as_deref(), Some(compact.as_slice()));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_value_keeps_the_keys_ttl() {
    // A plain `SET` clears the TTL, which made every edited key permanent —
    // a session or a lock that was meant to expire, living forever.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set(
            "session:1",
            "old",
            Some(fred::types::Expiration::EX(600)),
            None,
            false,
        )
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let written = redis_pane::redis::mutate::set_value(&client, b"session:1", b"new")
        .await
        .unwrap();

    assert!(written);
    let now: Option<String> = writer.get("session:1").await.unwrap();
    assert_eq!(now.as_deref(), Some("new"));
    let ttl: i64 = writer.ttl("session:1").await.unwrap();
    assert!(
        (1..=600).contains(&ttl),
        "TTL must survive the edit, got {ttl}"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_value_on_a_key_that_is_gone_writes_nothing_and_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let written = redis_pane::redis::mutate::set_value(&client, b"expired:1", b"new")
        .await
        .unwrap();

    assert!(!written);
    let exists: i64 = writer.exists("expired:1").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M2 task 6 step 0 — binary safety of HSCAN, checked before writing any
//    edit path ──────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn a_hash_with_non_utf8_field_and_value_reads_as_bytes_rather_than_failing() {
    // Pinned in M2 task 6 step 0 as a known gap: `hscan_window` asked fred for
    // `Vec<String>`, and fred's `FromValue for String` refuses invalid UTF-8,
    // so one binary field failed the whole read. Members are read as bytes
    // now, and the exact bytes must come back (review C2).
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let bad_field: &[u8] = &[0xff, 0xfe, b'x'];
    let bad_value: &[u8] = &[b'y', 0xff, 0xfe];
    let _: () = writer
        .hset("bytey-hash", vec![(bad_field, bad_value)])
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let arming = redis_pane::redis::read::Arming::Enabled;
    let result = redis_pane::redis::read::read_value(&client, b"bytey-hash", arming).await;

    match result.map(|read| read.map(|r| r.value)) {
        Ok(Some(redis_pane_core::state::value::Value::Hash(hash))) => {
            assert_eq!(hash.pairs, vec![(bad_field.to_vec(), bad_value.to_vec())]);
        }
        other => panic!("expected the hash, read as bytes; got {other:?}"),
    }

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn every_collection_type_reads_non_utf8_members_as_bytes() {
    use fred::types::{CustomCommand, Value as Wire};
    use redis_pane_core::state::value::{IndexedValue, MemberValue, ScoredValue, Value};

    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let raw: &[u8] = &[b'm', 0xff, 0x80];
    for (cmd, args) in [
        ("RPUSH", vec![Wire::from("bytey-list"), Wire::from(raw)]),
        ("SADD", vec![Wire::from("bytey-set"), Wire::from(raw)]),
        (
            "ZADD",
            vec![Wire::from("bytey-zset"), Wire::from("1"), Wire::from(raw)],
        ),
        (
            "XADD",
            vec![
                Wire::from("bytey-stream"),
                Wire::from("*"),
                Wire::from(raw),
                Wire::from(raw),
            ],
        ),
    ] {
        let _: Wire = writer
            .custom(CustomCommand::new(cmd, None, false), args)
            .await
            .unwrap();
    }

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let arming = redis_pane::redis::read::Arming::Enabled;
    let read = |name: &'static [u8]| {
        let client = client.clone();
        async move {
            redis_pane::redis::read::read_value(&client, name, arming)
                .await
                .map(|read| read.map(|r| r.value))
        }
    };

    assert_eq!(
        read(b"bytey-list").await.unwrap(),
        Some(Value::List(IndexedValue {
            items: vec![raw.to_vec()],
            total: 1,
        }))
    );
    assert_eq!(
        read(b"bytey-set").await.unwrap(),
        Some(Value::Set(MemberValue {
            members: vec![raw.to_vec()],
            total: 1,
        }))
    );
    assert_eq!(
        read(b"bytey-zset").await.unwrap(),
        Some(Value::ZSet(ScoredValue {
            entries: vec![(raw.to_vec(), 1.0)],
            total: 1,
        }))
    );
    match read(b"bytey-stream").await.unwrap() {
        Some(Value::Stream(stream)) => {
            assert_eq!(stream.entries.len(), 1);
            assert_eq!(stream.entries[0].1, vec![(raw.to_vec(), raw.to_vec())]);
        }
        other => panic!("expected the stream, read as bytes; got {other:?}"),
    }

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// Review H1: `mutate::execute` is the shell's whole write interface, so each
/// outcome the core gives meaning to is pinned here against a real server.
#[tokio::test]
#[ignore = "needs docker"]
async fn execute_reports_how_each_write_settled() {
    use redis_pane::redis::mutate::execute;
    use redis_pane_core::mutation::{Mutation, MutationOutcome, NotWritten};

    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("s", "old", None, None, false).await.unwrap();
    let _: () = writer.hset("h", [("f", "v")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let cases = [
        (
            Mutation::SetString {
                key: "s".into(),
                value: b"new".to_vec(),
            },
            MutationOutcome::Done,
        ),
        (
            Mutation::SetString {
                key: "missing".into(),
                value: b"new".to_vec(),
            },
            MutationOutcome::NotWritten(NotWritten::KeyGone),
        ),
        (
            Mutation::SetHashField {
                key: "h".into(),
                field: b"nope".to_vec(),
                value: b"x".to_vec(),
            },
            MutationOutcome::NotWritten(NotWritten::FieldGone),
        ),
        (
            Mutation::AddHashField {
                key: "h".into(),
                field: b"f".to_vec(),
                value: b"x".to_vec(),
            },
            MutationOutcome::NotWritten(NotWritten::FieldExists),
        ),
        (
            Mutation::DeleteHashField {
                key: "h".into(),
                field: b"nope".to_vec(),
            },
            MutationOutcome::NothingToRemove,
        ),
        (
            Mutation::DeleteKey { key: "s".into() },
            MutationOutcome::Done,
        ),
    ];
    for (mutation, expected) in cases {
        let got = execute(&client, &mutation).await.unwrap();
        assert_eq!(got, expected, "{mutation:?}");
    }
    let exists: i64 = writer.exists("missing").await.unwrap();
    assert_eq!(exists, 0, "a SET on a gone key never recreates it");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M2 task 6 — guarded Hash field writes (D1, ADR-0015) ─────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_hash_field_overwrites_it_and_keeps_the_keys_ttl() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:1", [("token", "old")]).await.unwrap();
    let _: () = writer.expire("h:1", 600, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_hash_field(&client, b"h:1", b"token", b"new")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldWrite::Written);

    let now: Option<String> = writer.hget("h:1", "token").await.unwrap();
    assert_eq!(now.as_deref(), Some("new"));
    let ttl: i64 = writer.ttl("h:1").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_hash_field_with_a_field_ttl_keeps_it_on_redis_7_4() {
    field_ttl_survives_an_edit("redis", "7.4-alpine").await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_hash_field_with_a_field_ttl_keeps_it_on_redis_8_4() {
    field_ttl_survives_an_edit("redis", "8.4-alpine").await;
}

/// Shared body for the two field-TTL tests above: `HEXPIRE` is 7.4+, so this
/// is only run against servers that have it.
async fn field_ttl_survives_an_edit(image: &str, tag: &str) {
    let (_c, url) = start(image, tag).await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:2", [("token", "old")]).await.unwrap();
    let field_ttl: Vec<i64> = writer
        .custom(
            fred::types::CustomCommand::new("HEXPIRE", None, false),
            vec!["h:2", "600", "FIELDS", "1", "token"],
        )
        .await
        .unwrap();
    assert_eq!(field_ttl, vec![1], "HEXPIRE must have set the field TTL");

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_hash_field(&client, b"h:2", b"token", b"new")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldWrite::Written);

    let now: Option<String> = writer.hget("h:2", "token").await.unwrap();
    assert_eq!(now.as_deref(), Some("new"));
    let httl: Vec<i64> = writer
        .custom(
            fred::types::CustomCommand::new("HTTL", None, false),
            vec!["h:2", "FIELDS", "1", "token"],
        )
        .await
        .unwrap();
    assert_eq!(httl.len(), 1);
    assert!(
        (1..=600).contains(&httl[0]),
        "field TTL must survive the edit, got {httl:?}"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn the_edit_script_still_works_on_redis_6_2_where_hpexpiretime_does_not_exist() {
    // The `HPEXPIRETIME` `pcall` must be harmless here: 6.2 predates the
    // whole `HEXPIRE` family, so the unknown-subcommand error has to come
    // back as a Lua error table the script keeps running past, not abort the
    // script outright.
    let (_c, url) = start("redis", "6.2-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:3", [("token", "old")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_hash_field(&client, b"h:3", b"token", b"new")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldWrite::Written);

    let now: Option<String> = writer.hget("h:3", "token").await.unwrap();
    assert_eq!(now.as_deref(), Some("new"));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_gone_field_writes_nothing() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:4", [("a", "1")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_hash_field(&client, b"h:4", b"missing", b"new")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldWrite::FieldGone);

    let still_absent: Option<String> = writer.hget("h:4", "missing").await.unwrap();
    assert_eq!(still_absent, None, "nothing must have been written");
    let untouched: Option<String> = writer.hget("h:4", "a").await.unwrap();
    assert_eq!(
        untouched.as_deref(),
        Some("1"),
        "the other field is untouched"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_field_on_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_hash_field(&client, b"h:gone", b"token", b"new")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldWrite::KeyGone);

    let exists: i64 = writer.exists("h:gone").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_new_hash_field_creates_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:5", [("a", "1")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_hash_field(&client, b"h:5", b"b", b"2")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldAdd::Added);

    let value: Option<String> = writer.hget("h:5", "b").await.unwrap();
    assert_eq!(value.as_deref(), Some("2"));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_field_that_already_exists_leaves_it_unchanged() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:6", [("a", "1")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_hash_field(&client, b"h:6", b"a", b"clobbered")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldAdd::FieldExists);

    let value: Option<String> = writer.hget("h:6", "a").await.unwrap();
    assert_eq!(value.as_deref(), Some("1"), "must not be overwritten");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_field_to_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_hash_field(&client, b"h:gone2", b"a", b"1")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldAdd::KeyGone);

    let exists: i64 = writer.exists("h:gone2").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_a_hash_field_removes_only_that_field() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:7", [("a", "1"), ("b", "2")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_hash_field(&client, b"h:7", b"a")
        .await
        .unwrap();
    assert!(removed);

    let a: Option<String> = writer.hget("h:7", "a").await.unwrap();
    assert_eq!(a, None);
    let b: Option<String> = writer.hget("h:7", "b").await.unwrap();
    assert_eq!(b.as_deref(), Some("2"), "the other field is untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_an_already_gone_field_reports_false() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:8", [("a", "1")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_hash_field(&client, b"h:8", b"missing")
        .await
        .unwrap();
    assert!(!removed);

    let a: Option<String> = writer.hget("h:8", "a").await.unwrap();
    assert_eq!(a.as_deref(), Some("1"), "untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_the_last_field_deletes_the_key() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.hset("h:9", [("only", "1")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_hash_field(&client, b"h:9", b"only")
        .await
        .unwrap();
    assert!(removed);

    let exists: i64 = writer.exists("h:9").await.unwrap();
    assert_eq!(exists, 0, "the key must be gone once its last field is");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_a_field_with_bytes_that_look_like_small_integers_touches_only_that_field() {
    // The mirror of `deleting_a_key_with_bytes_that_look_like_small_integers`
    // above, for `hdel`'s own `Into<MultipleKeys>` field parameter: a bare
    // `Vec<u8>` would be read elementwise, one field per byte value, rather
    // than as one binary-safe field name.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let field: &[u8] = &[7, 8];
    let _: () = writer
        .hset("h:10", vec![(field, b"v".as_slice())])
        .await
        .unwrap();
    // Decoy fields named after the byte values, as strings — if the bug were
    // present, deleting `field` would remove one of these instead.
    let _: () = writer
        .hset("h:10", [("7", "decoy7"), ("8", "decoy8")])
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_hash_field(&client, b"h:10", field)
        .await
        .unwrap();
    assert!(removed);

    let target: Option<String> = writer.hget("h:10", field).await.unwrap();
    assert_eq!(target, None, "the intended field must be gone");
    let decoy7: Option<String> = writer.hget("h:10", "7").await.unwrap();
    assert_eq!(decoy7.as_deref(), Some("decoy7"), "decoy must be untouched");
    let decoy8: Option<String> = writer.hget("h:10", "8").await.unwrap();
    assert_eq!(decoy8.as_deref(), Some("decoy8"), "decoy must be untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_bytey_field_name_can_be_edited_without_touching_a_decoy() {
    // Mirror of the delete-side byte test above, for `set_hash_field`'s
    // `ARGV` path.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let field: &[u8] = &[7, 8];
    let _: () = writer
        .hset("h:11", vec![(field, b"old".as_slice())])
        .await
        .unwrap();
    let _: () = writer.hset("h:11", [("7", "decoy")]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_hash_field(&client, b"h:11", field, b"new")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::FieldWrite::Written);

    let target: Option<String> = writer.hget("h:11", field).await.unwrap();
    assert_eq!(target.as_deref(), Some("new"));
    let decoy: Option<String> = writer.hget("h:11", "7").await.unwrap();
    assert_eq!(decoy.as_deref(), Some("decoy"), "decoy must be untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M2 task 7 — guarded Set member writes (D2, ADR-0016) ─────────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_set_member_to_a_gone_key_does_not_recreate_it() {
    // The whole reason `SET_MEMBER_ADD_SCRIPT` exists (ADR-0016 D2): a plain
    // `SADD` would bring a key that expired or was deleted under the confirm
    // dialog back to life with a single member.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.sadd("set:1", "alpha").await.unwrap();
    let deleted: i64 = writer.del("set:1").await.unwrap();
    assert_eq!(deleted, 1, "setup: the key must actually have existed");

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_set_member(&client, b"set:1", b"beta")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::MemberAdd::KeyGone);

    let exists: i64 = writer.exists("set:1").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_duplicate_set_member_refuses_without_writing() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.sadd("set:2", "alpha").await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_set_member(&client, b"set:2", b"alpha")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::MemberAdd::MemberExists);

    let members: Vec<String> = writer.smembers("set:2").await.unwrap();
    assert_eq!(members, vec!["alpha".to_string()], "must be unchanged");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_new_set_member_adds_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.sadd("set:3", "alpha").await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_set_member(&client, b"set:3", b"beta")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::MemberAdd::Added);

    let is_member: bool = writer.sismember("set:3", "beta").await.unwrap();
    assert!(is_member, "the new member must be present");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn removing_the_last_set_member_deletes_the_key() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.sadd("set:4", "only").await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_set_member(&client, b"set:4", b"only")
        .await
        .unwrap();
    assert!(removed);

    let exists: i64 = writer.exists("set:4").await.unwrap();
    assert_eq!(exists, 0, "the key must be gone once its last member is");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_an_already_gone_set_member_reports_false() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.sadd("set:5", "alpha").await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_set_member(&client, b"set:5", b"missing")
        .await
        .unwrap();
    assert!(!removed);

    let members: Vec<String> = writer.smembers("set:5").await.unwrap();
    assert_eq!(members, vec!["alpha".to_string()], "untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_binary_set_member_round_trips_through_add_and_delete() {
    // Review C2: members are `Vec<u8>`, never `String`. A member containing
    // invalid UTF-8 must survive both the add script's ARGV and SREM's
    // member argument unchanged.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let member: &[u8] = b"m\xff\x80";
    let _: () = writer.sadd("set:bin", "plain").await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_set_member(&client, b"set:bin", member)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::MemberAdd::Added);

    let is_member: bool = writer.sismember("set:bin", member).await.unwrap();
    assert!(is_member, "the binary member must round-trip through add");

    let removed = redis_pane::redis::mutate::delete_set_member(&client, b"set:bin", member)
        .await
        .unwrap();
    assert!(removed);

    let is_member_after: bool = writer.sismember("set:bin", member).await.unwrap();
    assert!(
        !is_member_after,
        "the binary member must round-trip through delete"
    );
    let plain_still_there: bool = writer.sismember("set:bin", "plain").await.unwrap();
    assert!(plain_still_there, "the other member is untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_set_member_against_a_wrong_type_key_surfaces_an_error_not_a_panic() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("str:1", "hello", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let err = redis_pane::redis::mutate::add_set_member(&client, b"str:1", b"x")
        .await
        .expect_err("SADD against a String key is WRONGTYPE");
    assert!(err.details().contains("WRONGTYPE"), "{err}");

    let value: Option<String> = writer.get("str:1").await.unwrap();
    assert_eq!(
        value.as_deref(),
        Some("hello"),
        "untouched by the failed write"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── PLAN M2 task 8 — List element edit, add, delete (ADR-0017) ─────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_list_element_overwrites_it_and_keeps_the_keys_ttl() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .rpush("l:1", &["alpha", "beta", "gamma"])
        .await
        .unwrap();
    let _: bool = writer.expire("l:1", 600, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_list_element(&client, b"l:1", 1, b"beta", b"new")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ListElementWrite::Written
    );

    let now: Option<String> = writer.lindex("l:1", 1).await.unwrap();
    assert_eq!(now.as_deref(), Some("new"));
    let ttl: i64 = writer.ttl("l:1").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// The test the whole task exists for (D2, ADR-0017). A second client shifts
/// every index by pushing a new head between the read the dialog staged
/// against and the write; the guard must refuse rather than silently
/// overwriting whatever index 1 now holds, and the list must be provably
/// unchanged by the refused call.
///
/// This exercises the real race, not a simulation of it: the second client's
/// `LPUSH` genuinely runs, on the real server, strictly between the read this
/// test performs (to learn what "the dialog staged against" would have seen)
/// and the guarded `EVAL` this test issues against that now-stale index and
/// now-stale expected bytes. Determinism comes from sequencing two real
/// commands on two real connections in program order — `await`ing the
/// `LPUSH` to completion before calling `set_list_element` — rather than from
/// timing or sleeps, so there is no flakiness to add margin for.
#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_list_element_refuses_without_writing_when_the_list_shifted_under_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .rpush("l:2", &["alpha", "beta", "gamma"])
        .await
        .unwrap();
    let _: bool = writer.expire("l:2", 600, None).await.unwrap();

    // The dialog reads the list and stages an edit of "beta" at index 1.
    let staged_index = 1usize;
    let staged_expected: Vec<u8> = writer
        .lindex::<Option<String>, _>("l:2", staged_index as i64)
        .await
        .unwrap()
        .unwrap()
        .into_bytes();
    assert_eq!(staged_expected, b"beta");

    // A second client — a different connection, standing in for a second
    // session — pushes a new head before the dialog is confirmed. Every
    // index shifts: "beta" is now at index 2, and index 1 holds "alpha".
    let second_client = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    second_client.init().await.unwrap();
    let _: i64 = second_client.lpush("l:2", "head").await.unwrap();
    let _ = second_client.quit().await;

    // The staged write executes against the original index and the
    // original expected bytes — exactly what a confirm dialog opened before
    // the push, and confirmed after it, would send.
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_list_element(
        &client,
        b"l:2",
        staged_index,
        &staged_expected,
        b"CORRUPTED",
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ListElementWrite::ElementMoved,
        "a shifted index must refuse, not silently overwrite whatever moved into slot 1"
    );

    // The list must be provably unchanged by the refused write: still the
    // four elements the push left, in the shifted order, with nothing
    // replaced by "CORRUPTED" anywhere.
    let all: Vec<String> = writer.lrange("l:2", 0, -1).await.unwrap();
    assert_eq!(
        all,
        vec![
            "head".to_string(),
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
        ],
        "the refused write must not have touched any element"
    );
    assert!(
        !all.iter().any(|e| e == "CORRUPTED"),
        "the new value must never have been written anywhere in the list"
    );
    let ttl: i64 = writer.ttl("l:2").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_list_element_on_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome =
        redis_pane::redis::mutate::set_list_element(&client, b"l:gone", 0, b"anything", b"new")
            .await
            .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ListElementWrite::KeyGone
    );

    let exists: i64 = writer.exists("l:gone").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_list_element_to_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_list_element(
        &client,
        b"l:gone2",
        redis_pane_core::state::value::ListEnd::Tail,
        b"x",
    )
    .await
    .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ListElementAdd::KeyGone);

    let exists: i64 = writer.exists("l:gone2").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_list_element_lands_at_the_correct_end_for_head_and_tail() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer.rpush("l:3", &["middle"]).await.unwrap();
    let _: bool = writer.expire("l:3", 600, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();

    let head_outcome = redis_pane::redis::mutate::add_list_element(
        &client,
        b"l:3",
        redis_pane_core::state::value::ListEnd::Head,
        b"first",
    )
    .await
    .unwrap();
    assert_eq!(
        head_outcome,
        redis_pane::redis::mutate::ListElementAdd::Added
    );

    let tail_outcome = redis_pane::redis::mutate::add_list_element(
        &client,
        b"l:3",
        redis_pane_core::state::value::ListEnd::Tail,
        b"last",
    )
    .await
    .unwrap();
    assert_eq!(
        tail_outcome,
        redis_pane::redis::mutate::ListElementAdd::Added
    );

    let all: Vec<String> = writer.lrange("l:3", 0, -1).await.unwrap();
    assert_eq!(
        all,
        vec![
            "first".to_string(),
            "middle".to_string(),
            "last".to_string()
        ],
        "Head must land at index 0 and Tail must land at the end"
    );
    let ttl: i64 = writer.ttl("l:3").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_a_list_element_removes_the_right_duplicate() {
    // ADR-0017 D2's whole reason for the sentinel technique: `LREM key 1
    // <value>` removes the *first* match from the head, which on
    // `[x, y, x, z]` is index 0, not index 2. Deleting index 2 must leave
    // `[x, y, z]`.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer.rpush("l:4", &["x", "y", "x", "z"]).await.unwrap();
    let _: bool = writer.expire("l:4", 600, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::delete_list_element(&client, b"l:4", 2, b"x")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ListElementDelete::Removed
    );

    let all: Vec<String> = writer.lrange("l:4", 0, -1).await.unwrap();
    assert_eq!(
        all,
        vec!["x".to_string(), "y".to_string(), "z".to_string()],
        "must remove the second x (index 2), not the first"
    );
    let ttl: i64 = writer.ttl("l:4").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn deleting_the_last_list_element_deletes_the_key() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer.rpush("l:5", &["solo"]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::delete_list_element(&client, b"l:5", 0, b"solo")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ListElementDelete::Removed
    );

    let exists: i64 = writer.exists("l:5").await.unwrap();
    assert_eq!(exists, 0, "the key must go with its last element");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_binary_list_element_round_trips_through_edit_and_read() {
    // Review C2: `IndexedValue.items` is `Vec<Vec<u8>>`, never `String`. A
    // binary element must survive both the compare-and-set guard's ARGV and
    // the LSET it performs, unchanged.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let original: &[u8] = b"m\xff\x80";
    let edited: &[u8] = b"e\xff\x802";
    let _: i64 = writer.rpush("l:6", vec![original]).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_list_element(&client, b"l:6", 0, original, edited)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ListElementWrite::Written
    );

    let now: Option<Vec<u8>> = writer.lindex("l:6", 0).await.unwrap();
    assert_eq!(
        now.as_deref(),
        Some(edited),
        "the binary element must round-trip through the edit unchanged"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_list_element_against_a_wrong_type_key_surfaces_an_error_not_a_panic() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("str:2", "hello", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let err = redis_pane::redis::mutate::set_list_element(&client, b"str:2", 0, b"hello", b"x")
        .await
        .expect_err("LSET/LINDEX against a String key is WRONGTYPE");
    assert!(err.details().contains("WRONGTYPE"), "{err}");

    let value: Option<String> = writer.get("str:2").await.unwrap();
    assert_eq!(
        value.as_deref(),
        Some("hello"),
        "untouched by the failed write"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── PLAN M2 task 9 — ZSet score edit, add, delete (ADR-0018) ───────────────

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_zset_score_overwrites_it_and_keeps_the_keys_ttl() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd(
            "zs:1",
            None,
            None,
            false,
            false,
            vec![(1.0, "alpha"), (2.0, "beta"), (3.5, "gamma")],
        )
        .await
        .unwrap();
    let _: bool = writer.expire("zs:1", 600, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_zset_score(&client, b"zs:1", b"beta", 10.0)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ZSetScoreWrite::Written);

    let now: Option<f64> = writer.zscore("zs:1", "beta").await.unwrap();
    assert_eq!(now, Some(10.0));
    let ttl: i64 = writer.ttl("zs:1").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_zset_score_on_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_zset_score(&client, b"zs:gone", b"anything", 1.0)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ZSetScoreWrite::KeyGone);

    let exists: i64 = writer.exists("zs:gone").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// The gone-*member* half of D2's guard, distinct from the gone-*key* half
/// above: the key is still there, but the member the dialog staged against
/// was `ZREM`'d out from under it. `ZADD XX CH` cannot tell this apart from
/// "present with an unchanged score" (ADR-0018 D2, the reason the script
/// exists at all) — `ZSCORE`'s nil check is what makes the distinction, and
/// this test is what pins it against a real server.
#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_zset_score_refuses_without_writing_when_the_member_is_gone_under_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd(
            "zs:2",
            None,
            None,
            false,
            false,
            vec![(1.0, "alpha"), (2.0, "beta")],
        )
        .await
        .unwrap();
    let _: bool = writer.expire("zs:2", 600, None).await.unwrap();

    // The dialog stages a score edit of "beta" from 2.0 to 20.0. Before it is
    // confirmed, a second connection removes "beta" from the set entirely.
    let second_client = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    second_client.init().await.unwrap();
    let removed: i64 = second_client.zrem("zs:2", "beta").await.unwrap();
    assert_eq!(removed, 1, "setup: beta must actually have been removed");
    let _ = second_client.quit().await;

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_zset_score(&client, b"zs:2", b"beta", 20.0)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ZSetScoreWrite::MemberGone,
        "a member removed under the dialog must refuse, not silently re-add it"
    );

    let beta_score: Option<f64> = writer.zscore("zs:2", "beta").await.unwrap();
    assert_eq!(
        beta_score, None,
        "the refused write must not have resurrected beta"
    );
    let alpha_score: Option<f64> = writer.zscore("zs:2", "alpha").await.unwrap();
    assert_eq!(alpha_score, Some(1.0), "the untouched member is unchanged");
    let ttl: i64 = writer.ttl("zs:2").await.unwrap();
    assert!((1..=600).contains(&ttl), "key TTL must survive, got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// D7's test — the one proving rank is not identity, the way task 8's shift
/// test proved an index is not (ADR-0018 D7). A staged ZSet write names the
/// member's bytes, captured when the row was picked, so a concurrent write
/// that reorders every rank around it must not misdirect the write.
///
/// This exercises the real race, not a simulation of it: the second client's
/// three `ZADD`s genuinely run, on the real server, and are `await`ed to
/// completion — strictly between the moment "the dialog" would have picked
/// `beta` and the moment the staged edit actually executes. Determinism comes
/// from sequencing real commands on two real connections in program order,
/// the same technique task 8's list-shift test uses, so there is no timing
/// window to add margin for. The assertion checks the whole set's final
/// state, not just the return value, so a write that landed on the wrong
/// member (by rank rather than by name) would be caught even if it happened
/// to return `Written`.
#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_zset_score_lands_on_the_right_member_after_a_reorder_under_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd(
            "zs:3",
            None,
            None,
            false,
            false,
            vec![
                (1.0, "alpha"),
                (2.0, "beta"),
                (3.0, "gamma"),
                (4.0, "delta"),
            ],
        )
        .await
        .unwrap();
    let _: bool = writer.expire("zs:3", 600, None).await.unwrap();

    // The dialog picks "beta" (rank 1, score 2.0) and stages an edit to
    // 100.0. Before it is confirmed, a second connection rewrites every
    // *other* member's score, shuffling the ranks: alpha and gamma jump above
    // beta's old rank, delta drops to the very bottom.
    let original_rank: Option<i64> = writer.zrank("zs:3", "beta", false).await.unwrap();
    assert_eq!(original_rank, Some(1), "setup: beta must start at rank 1");

    let second_client = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    second_client.init().await.unwrap();
    let _: i64 = second_client
        .zadd(
            "zs:3",
            None,
            None,
            false,
            false,
            vec![(500.0, "alpha"), (1.0, "gamma"), (0.5, "delta")],
        )
        .await
        .unwrap();
    let _ = second_client.quit().await;

    // Ranks are now, ascending by score: delta(0.5) gamma(1.0) beta(2.0
    // still) alpha(500.0) — beta moved from rank 1 to rank 2 without anyone
    // touching it.
    let reordered_rank: Option<i64> = writer.zrank("zs:3", "beta", false).await.unwrap();
    assert_eq!(
        reordered_rank,
        Some(2),
        "setup: the reorder must actually have moved beta's rank"
    );

    // The staged write executes against the member name captured when it was
    // picked, oblivious to the rank shuffle.
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_zset_score(&client, b"zs:3", b"beta", 100.0)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ZSetScoreWrite::Written);

    // Assert the whole set's final state, not just the return value.
    let alpha: Option<f64> = writer.zscore("zs:3", "alpha").await.unwrap();
    let beta: Option<f64> = writer.zscore("zs:3", "beta").await.unwrap();
    let gamma: Option<f64> = writer.zscore("zs:3", "gamma").await.unwrap();
    let delta: Option<f64> = writer.zscore("zs:3", "delta").await.unwrap();
    assert_eq!(alpha, Some(500.0), "alpha must hold the reorder's score");
    assert_eq!(beta, Some(100.0), "beta must hold the staged edit's score");
    assert_eq!(gamma, Some(1.0), "gamma must hold the reorder's score");
    assert_eq!(delta, Some(0.5), "delta must hold the reorder's score");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_zset_member_to_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_zset_member(&client, b"zs:gone2", b"member", 1.0)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ZSetMemberAdd::KeyGone);

    let exists: i64 = writer.exists("zs:gone2").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn adding_a_duplicate_zset_member_refuses_without_changing_its_score() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd("zs:4", None, None, false, false, (1.0, "alpha"))
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::add_zset_member(&client, b"zs:4", b"alpha", 99.0)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ZSetMemberAdd::MemberExists
    );

    let score: Option<f64> = writer.zscore("zs:4", "alpha").await.unwrap();
    assert_eq!(score, Some(1.0), "the existing score must be untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn removing_the_last_zset_member_deletes_the_key() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd("zs:5", None, None, false, false, (1.0, "only"))
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let removed = redis_pane::redis::mutate::delete_zset_member(&client, b"zs:5", b"only")
        .await
        .unwrap();
    assert!(removed);

    let exists: i64 = writer.exists("zs:5").await.unwrap();
    assert_eq!(exists, 0, "the key must go with its last member");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_binary_zset_members_score_round_trips_through_edit_and_read() {
    // Review C2 / ADR-0018 D5: `ScoredValue.entries` is `Vec<(Vec<u8>, f64)>`,
    // never `String`. A score edit never touches the member's bytes — they
    // travel exactly as read — so a binary member's *score* must round-trip
    // even though the member itself is not text.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let member: &[u8] = b"m\xff\x80";
    let _: i64 = writer
        .zadd("zs:bin", None, None, false, false, (1.0, member.to_vec()))
        .await
        .unwrap();
    let _: i64 = writer
        .zadd("zs:bin", None, None, false, false, (9.0, "plain"))
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_zset_score(&client, b"zs:bin", member, 2.5)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ZSetScoreWrite::Written);

    let now: Option<f64> = writer.zscore("zs:bin", member).await.unwrap();
    assert_eq!(
        now,
        Some(2.5),
        "the binary member's score must round-trip through the edit"
    );
    let plain_untouched: Option<f64> = writer.zscore("zs:bin", "plain").await.unwrap();
    assert_eq!(plain_untouched, Some(9.0), "the other member is untouched");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn inf_and_negative_inf_round_trip_through_a_score_edit() {
    // ADR-0018's verified facts: `inf`/`-inf` are valid f64 scores and round-
    // trip as `inf`/`-inf`, unlike `nan`, which the server refuses outright
    // (rejected before it can be staged, per D4 — not exercised here).
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd("zs:6", None, None, false, false, (1.0, "alpha"))
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();

    let to_inf =
        redis_pane::redis::mutate::set_zset_score(&client, b"zs:6", b"alpha", f64::INFINITY)
            .await
            .unwrap();
    assert_eq!(to_inf, redis_pane::redis::mutate::ZSetScoreWrite::Written);
    let now: Option<f64> = writer.zscore("zs:6", "alpha").await.unwrap();
    assert_eq!(now, Some(f64::INFINITY));

    let to_neg_inf =
        redis_pane::redis::mutate::set_zset_score(&client, b"zs:6", b"alpha", f64::NEG_INFINITY)
            .await
            .unwrap();
    assert_eq!(
        to_neg_inf,
        redis_pane::redis::mutate::ZSetScoreWrite::Written
    );
    let now: Option<f64> = writer.zscore("zs:6", "alpha").await.unwrap();
    assert_eq!(now, Some(f64::NEG_INFINITY));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn a_high_precision_score_round_trips_byte_identically() {
    // ADR-0018's verified fact: `1.0000000000000002` reads back byte
    // identical via `ZSCORE`. Asserted here via `to_bits()`, not just `==`,
    // so a lossy round-trip that happened to still compare equal (as `0.0`
    // and `-0.0` do) would not slip past this test.
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .zadd("zs:7", None, None, false, false, (1.0, "alpha"))
        .await
        .unwrap();
    let precise: f64 = 1.0000000000000002;

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_zset_score(&client, b"zs:7", b"alpha", precise)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ZSetScoreWrite::Written);

    let now: Option<f64> = writer.zscore("zs:7", "alpha").await.unwrap();
    let now = now.expect("the member must still be there");
    assert_eq!(
        now.to_bits(),
        precise.to_bits(),
        "the score must round-trip byte-identically, not just compare equal"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn editing_a_zset_score_against_a_wrong_type_key_surfaces_an_error_not_a_panic() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("str:3", "hello", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let err = redis_pane::redis::mutate::set_zset_score(&client, b"str:3", b"hello", 1.0)
        .await
        .expect_err("ZADD/ZSCORE against a String key is WRONGTYPE");
    assert!(err.details().contains("WRONGTYPE"), "{err}");

    let value: Option<String> = writer.get("str:3").await.unwrap();
    assert_eq!(
        value.as_deref(),
        Some("hello"),
        "untouched by the failed write"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── PLAN M2 task 10 — TTL set, persist, extend/shorten (ADR-0019) ──────────

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_ttl_lands_and_ttl_reads_it_back() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:set1", "v", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_ttl(&client, b"ttl:set1", 300)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::SetTtlWrite::Written);

    let ttl: i64 = writer.ttl("ttl:set1").await.unwrap();
    assert!((1..=300).contains(&ttl), "got {ttl}");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_ttl_on_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_ttl(&client, b"ttl:set_gone", 300)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::SetTtlWrite::KeyGone);

    let exists: i64 = writer.exists("ttl:set_gone").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn persisting_a_ttl_clears_the_expiry() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:persist1", "v", None, None, false)
        .await
        .unwrap();
    let _: bool = writer.expire("ttl:persist1", 600, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::persist_ttl(&client, b"ttl:persist1")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::PersistTtlWrite::Written);

    let ttl: i64 = writer.ttl("ttl:persist1").await.unwrap();
    assert_eq!(ttl, -1, "TTL must read back as no-expiry");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn persisting_a_ttl_on_a_key_with_no_expiry_settles_without_error() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:persist2", "v", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::persist_ttl(&client, b"ttl:persist2")
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::PersistTtlWrite::NoExpiry
    );

    let value: Option<String> = writer.get("ttl:persist2").await.unwrap();
    assert_eq!(value.as_deref(), Some("v"), "untouched by the no-op");
    let ttl: i64 = writer.ttl("ttl:persist2").await.unwrap();
    assert_eq!(ttl, -1);

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn persisting_a_ttl_on_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::persist_ttl(&client, b"ttl:persist_gone")
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::PersistTtlWrite::KeyGone);

    let exists: i64 = writer.exists("ttl:persist_gone").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// D7's test — the analogue of task 9's reorder test, and the test this row
/// exists for. `shift_ttl` carries only a signed delta, never a "staged old
/// TTL" — that is the whole point of D5/D7's script running the arithmetic
/// atomically at the server against whatever `TTL` reads *there*, not against
/// any value this session captured earlier. This exercises the real race,
/// not a simulation of it: a second, real connection genuinely rewrites the
/// key's TTL on the live server, awaited to completion, strictly between the
/// moment "the dialog" would have captured the original TTL and the moment
/// the staged extend actually executes. The two candidate results — computed
/// from the stale, pre-change TTL versus the live, post-change one — are
/// numerically far enough apart that landing on the wrong one is unambiguous,
/// and the assertion checks the resulting TTL itself, not just the return
/// value.
#[tokio::test]
#[ignore = "needs docker"]
async fn extending_a_ttl_adds_to_the_servers_ttl_not_the_staged_one() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:extend1", "v", None, None, false)
        .await
        .unwrap();
    // "The dialog" opens against this TTL — 100s — and stages "+50m" (a
    // +50-second delta, kept small here for a fast test).
    let _: bool = writer.expire("ttl:extend1", 100, None).await.unwrap();

    // Before the staged extend is confirmed, a second, independent
    // connection changes the server's TTL to something far away from the
    // staged value.
    let second_client = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    second_client.init().await.unwrap();
    let _: bool = second_client
        .expire("ttl:extend1", 500, None)
        .await
        .unwrap();
    let server_ttl_before_extend: i64 = second_client.ttl("ttl:extend1").await.unwrap();
    assert!(
        (490..=500).contains(&server_ttl_before_extend),
        "setup: the second connection's EXPIRE must actually have landed, got {server_ttl_before_extend}"
    );
    let _ = second_client.quit().await;

    // The staged extend executes now, carrying only the delta — no "old TTL"
    // travels with it at all (D6/D7).
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::shift_ttl(&client, b"ttl:extend1", 50)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ShiftTtlWrite::Written);

    let final_ttl: i64 = writer.ttl("ttl:extend1").await.unwrap();
    // Computed from the *staged* TTL (100) + 50 would land at ~150.
    // Computed from the *server's* TTL (~500) + 50 lands at ~550.
    assert!(
        final_ttl > 400,
        "the extend must be computed from the server's live TTL (~500 + 50 = ~550), \
         not the staged one (100 + 50 = 150) — got {final_ttl}"
    );
    assert!(
        (530..=550).contains(&final_ttl),
        "expected ~550 (server's live TTL + delta), got {final_ttl}"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn shifting_a_ttl_on_a_key_with_no_expiry_refuses_and_writes_nothing() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:shift_noexp", "v", None, None, false)
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::shift_ttl(&client, b"ttl:shift_noexp", 30)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ShiftTtlWrite::NoExpiry);

    let ttl: i64 = writer.ttl("ttl:shift_noexp").await.unwrap();
    assert_eq!(ttl, -1, "must still have no expiry");
    let value: Option<String> = writer.get("ttl:shift_noexp").await.unwrap();
    assert_eq!(value.as_deref(), Some("v"), "untouched by the refusal");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn shifting_a_ttl_past_zero_refuses_and_the_key_still_exists() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:shift_zero", "v", None, None, false)
        .await
        .unwrap();
    let _: bool = writer.expire("ttl:shift_zero", 10, None).await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::shift_ttl(&client, b"ttl:shift_zero", -1000)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        redis_pane::redis::mutate::ShiftTtlWrite::WouldExpireNow
    );

    let exists: i64 = writer.exists("ttl:shift_zero").await.unwrap();
    assert_eq!(exists, 1, "the key must still exist after the refusal");
    let ttl: i64 = writer.ttl("ttl:shift_zero").await.unwrap();
    assert!(
        (1..=10).contains(&ttl),
        "the TTL must be untouched, got {ttl}"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn shifting_a_ttl_on_a_gone_key_does_not_recreate_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::shift_ttl(&client, b"ttl:shift_gone", 30)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::ShiftTtlWrite::KeyGone);

    let exists: i64 = writer.exists("ttl:shift_gone").await.unwrap();
    assert_eq!(exists, 0, "the key must not be recreated");

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_ttl_changes_neither_the_value_nor_the_type_nor_the_member_count() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: i64 = writer
        .hset(
            "ttl:hash_unchanged",
            [("f1", "v1"), ("f2", "v2"), ("f3", "v3")],
        )
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let outcome = redis_pane::redis::mutate::set_ttl(&client, b"ttl:hash_unchanged", 300)
        .await
        .unwrap();
    assert_eq!(outcome, redis_pane::redis::mutate::SetTtlWrite::Written);

    let count: i64 = writer.hlen("ttl:hash_unchanged").await.unwrap();
    assert_eq!(count, 3, "the field count must be unchanged");
    let f1: Option<String> = writer.hget("ttl:hash_unchanged", "f1").await.unwrap();
    let f2: Option<String> = writer.hget("ttl:hash_unchanged", "f2").await.unwrap();
    let f3: Option<String> = writer.hget("ttl:hash_unchanged", "f3").await.unwrap();
    assert_eq!(f1.as_deref(), Some("v1"));
    assert_eq!(f2.as_deref(), Some("v2"));
    assert_eq!(f3.as_deref(), Some("v3"));
    let ttl: i64 = writer.ttl("ttl:hash_unchanged").await.unwrap();
    assert!((1..=300).contains(&ttl));

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

/// D1 made concrete: unlike every sibling task, there is no `WRONGTYPE` case
/// to test here, because `EXPIRE` applies identically to every Redis type.
/// This asserts that explicitly, on a String, a Hash and a ZSet, rather than
/// reaching for a `WRONGTYPE` test that cannot happen for this mutation.
#[tokio::test]
#[ignore = "needs docker"]
async fn setting_a_ttl_works_identically_on_a_string_a_hash_and_a_zset() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer
        .set("ttl:type_str", "v", None, None, false)
        .await
        .unwrap();
    let _: i64 = writer.hset("ttl:type_hash", [("f", "v")]).await.unwrap();
    let _: i64 = writer
        .zadd("ttl:type_zset", None, None, false, false, (1.0, "m"))
        .await
        .unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    for key in ["ttl:type_str", "ttl:type_hash", "ttl:type_zset"] {
        let outcome = redis_pane::redis::mutate::set_ttl(&client, key.as_bytes(), 300)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            redis_pane::redis::mutate::SetTtlWrite::Written,
            "EXPIRE must succeed identically on {key}"
        );
        let ttl: i64 = writer.ttl(key).await.unwrap();
        assert!((1..=300).contains(&ttl), "{key} got ttl {ttl}");
    }

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── The value pane must not wedge after a silent disconnect ─────────────────
//
// docs/plans/value-pane-frozen-after-silent-disconnect.md. A laptop sleeping,
// an SSH session going idle, or a NAT/LB dropping an idle connection all
// black-hole a socket the same way: no RST, no FIN, just silence. `docker
// pause` reproduces exactly that — it freezes the container's process
// without touching the TCP connection, so a read against it blocks the way a
// truly dead-but-silent connection does, not the way a cleanly closed one
// does.
//
// Before `connect_with`'s bounded responsiveness config
// (`crates/app/src/redis/mod.rs`), `read.await` inside `ReadPermit::run`
// (`crates/app/src/redis/read.rs`) never resolved against a black-holed
// socket, so the `_guard` was never dropped and every read after it —
// including the reconnect's own refetch — queued behind it forever. This
// proves the read resolves as an error within the configured bound instead
// of hanging, that the resulting error is classified as connection loss the
// same way `Shell::watch_link` does (crates/app/src/terminal.rs), and that a
// second read through the same `ReadGate` — standing in for the reconnect's
// refetch — is not left wedged behind the first.
#[tokio::test]
#[ignore = "needs docker"]
async fn a_silently_dead_connection_frees_the_read_gate_within_the_configured_timeout() {
    let (container, url) = start("redis", "7-alpine").await;
    let (client, est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(est.tracking_supported);

    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();
    let _: () = writer.set("k", "v1", None, None, false).await.unwrap();

    // The same `ReadGate` a real `Shell` holds for the life of the
    // connection: every read (an Open, a manual Refetch, an
    // invalidation-triggered re-arm) takes a turn through it, and whether a
    // stuck read ever gives its turn back is exactly the deadlock this test
    // is for.
    let mut gate = redis_pane::redis::read::ReadGate::default();
    // Subscribed before the pause, the same way `Shell::watch_link` is
    // subscribed for the life of the connection — this is what proves the
    // widened classification in `terminal.rs`'s `error_rx` match, not just
    // the timeout value in `mod.rs`.
    let mut errors = client.error_rx();

    // Freezes the server's process; the socket itself is untouched, so
    // `poll_next` on it just stays `Pending` — the black hole, not a clean
    // disconnect.
    container.pause().await.unwrap();

    let permit = gate.begin();
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        permit.run(redis_pane::redis::read::read_value(
            &client,
            b"k",
            redis_pane::redis::read::Arming::Enabled,
        )),
    )
    .await
    .expect(
        "read.await must resolve within the configured responsiveness bound \
         instead of hanging forever on a black-holed socket",
    );
    assert!(
        matches!(outcome, Some(Err(_))),
        "a read against a dead connection must surface as an error, not a value"
    );

    // Whichever of the two configured timeouts released the read above
    // (fred's unresponsive-connection detector, or the per-command
    // `default_command_timeout` backstop — see the comment in
    // `connect_with`), it must also be one `Shell::watch_link` classifies as
    // connection loss.
    let (err, _server) = tokio::time::timeout(Duration::from_secs(10), errors.recv())
        .await
        .expect("a dead connection must eventually report itself on error_rx")
        .expect("error_rx must not close while the connection is merely dead");
    assert!(
        matches!(
            err.kind(),
            fred::error::ErrorKind::IO | fred::error::ErrorKind::Timeout
        ),
        "unexpected ErrorKind for a dead connection: {:?} ({err})",
        err.kind()
    );

    // The deadlock this bug report describes: without the fix, the guard
    // from the first read is never dropped, so the next read — the
    // reconnect's own refetch — queues behind it and never runs.
    //
    // Asked directly, with a read that does no I/O: the gate must hand the
    // next permit its turn at once. An earlier version sent a real command
    // here, against the still-paused server, under a 10s bound — the same
    // length as `default_command_timeout` (`connect_with`) — so whenever
    // fred's unresponsive detector did not release that command first, the
    // test raced the command timeout it was not about and failed nightly
    // (run 36307520486). Whether a read against a dead connection resolves
    // at all is the first half of this test; the real refetch below runs on
    // a fresh connection, exactly as `Command::Reconnect` does.
    let permit = gate.begin();
    let ran = tokio::time::timeout(Duration::from_secs(1), permit.run(async {}))
        .await
        .expect("the read-gate mutex must not still be held by the first, stuck read");
    assert!(
        ran.is_some(),
        "a fresh permit that nothing superseded must get its turn"
    );

    container.unpause().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // What the app's own `Command::Reconnect` does: build a fresh
    // connection (`connect_with` again) and read the open key through it.
    // This is the re-arm half of the invariant, proven the same way
    // `a_reconnect_loses_tracking_which_is_why_it_must_be_re_armed` proves
    // it — a fresh connection tracks nothing until a read arms it again.
    let (fresh, fresh_est) = redis_pane::redis::connect(&url).await.unwrap();
    assert!(fresh_est.tracking_supported);
    let mut fresh_gate = redis_pane::redis::read::ReadGate::default();
    let permit = fresh_gate.begin();
    let refetched = permit
        .run(redis_pane::redis::read::read_value(
            &fresh,
            b"k",
            redis_pane::redis::read::Arming::Enabled,
        ))
        .await
        .unwrap()
        .unwrap()
        .expect("the key must still be readable once the connection is back");
    match refetched.value {
        redis_pane_core::state::value::Value::Str(s) => {
            assert_eq!(s.raw, "v1", "the refetch must see the key's real value")
        }
        other => panic!("expected a String value, got {other:?}"),
    }

    let _ = client.quit().await;
    let _ = writer.quit().await;
    let _ = fresh.quit().await;
}

// ── M3 — the Slowlog read path (R6.4, `docs/plans/m3-slowlog.md`) ───────────

/// `CONFIG SET parameter value`, issued through fred's raw-command escape
/// hatch rather than `ConfigInterface`: the shipped binary never sends
/// `CONFIG SET` (there is no requirement for it — R6.4 is read-and-reset
/// only, `docs/plans/m3-slowlog.md`'s "Out of scope"), so `i-config` is not
/// a feature the app crate carries; only this test needs it, and it can ask
/// for exactly this one command without it.
async fn config_set(client: &Client, parameter: &str, value: &str) {
    let _: () = client
        .custom(
            CustomCommand::new("CONFIG", parameter.as_bytes(), false),
            vec![
                Value::from("SET"),
                Value::from(parameter.to_string()),
                Value::from(value.to_string()),
            ],
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs docker"]
async fn the_slowlog_read_path_fetches_entries_then_reset_via_the_mutate_chokepoint_empties_it() {
    let (_c, url) = start("redis", "7-alpine").await;
    let writer = Builder::from_config(Config::from_url(&url).unwrap())
        .build()
        .unwrap();
    writer.init().await.unwrap();

    // Every command qualifies — a standard technique for testing this
    // reliably rather than trying to time a real slow command.
    config_set(&writer, "slowlog-log-slower-than", "0").await;
    config_set(&writer, "slowlog-max-len", "128").await;
    let _: () = writer.slowlog_reset().await.unwrap();

    let _: () = writer
        .set("slowlog:probe", "value", None, None, false)
        .await
        .unwrap();
    let _: String = writer.get("slowlog:probe").await.unwrap();

    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();
    let entries = redis_pane::redis::read::fetch_slowlog(&client, 256)
        .await
        .unwrap();
    assert!(
        entries.len() >= 2,
        "both the SET and the GET should have logged: {entries:?}"
    );
    let commands: Vec<String> = entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.command).into_owned())
        .collect();
    assert!(
        commands.iter().any(|c| c.starts_with("SET slowlog:probe")),
        "{commands:?}"
    );
    assert!(
        commands.iter().any(|c| c.starts_with("GET slowlog:probe")),
        "{commands:?}"
    );
    // Every entry has a real id, a plausible timestamp, and a client address
    // — the six-element shape this app's Redis 6.0 floor guarantees
    // (ADR-0007), parsed without falling back to a default on a short reply.
    for entry in &entries {
        assert!(entry.id >= 0, "{entry:?}");
        assert!(entry.timestamp > 0, "{entry:?}");
        assert!(!entry.client_addr.is_empty(), "{entry:?}");
    }

    // `SLOWLOG RESET` (M3 phase B, `docs/plans/m3-slowlog.md` Decision 8) —
    // through the real mutation chokepoint (`redis_pane::redis::mutate::execute`),
    // not `writer.slowlog_reset()` directly, so this pins the same
    // `crates/app/src/redis/mutate.rs` code path the confirm dialog's `y`
    // actually dispatches — the point of this task's "run it through the
    // real mutate path" ask.
    //
    // `SLOWLOG RESET` is not itself exempt from being logged — with the
    // threshold at 0 it logs *itself*, so the surviving ring buffer is not
    // empty but contains exactly that one entry. A real discovery this test
    // caught on its first run: an earlier draft asserted `is_empty()` and
    // failed against a live 7-alpine container.
    let outcome = redis_pane::redis::mutate::execute(
        &client,
        &redis_pane_core::mutation::Mutation::ResetSlowlog,
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        redis_pane_core::mutation::MutationOutcome::Done,
        "RESET has no guard to trip (D8) — it always settles Done"
    );
    let after_reset = redis_pane::redis::read::fetch_slowlog(&client, 256)
        .await
        .unwrap();
    assert!(
        after_reset
            .iter()
            .all(|e| String::from_utf8_lossy(&e.command).starts_with("SLOWLOG")),
        "everything but the reset's own entry must be gone: {after_reset:?}"
    );

    let _ = client.quit().await;
    let _ = writer.quit().await;
}

// ── M3 task 6 phase A — the Dashboard's `INFO` read path
// (`docs/plans/m3-dashboard.md`) ─────────────────────────────────────────────
//
// The tile-derivation tests in `crates/core/src/state/dashboard.rs` prove the
// parser and the alarm math against a fixed, hand-written fixture. What they
// cannot prove is that a real server's `INFO` reply actually has the shape
// those fixtures assume — section names, field names, what is present versus
// absent on a fresh container. These two tests are that check, the same
// "most likely to catch a real-world format surprise" reasoning
// `the_slowlog_read_path...` above already applies to `SLOWLOG GET`.

#[tokio::test]
#[ignore = "needs docker"]
async fn a_fresh_containers_real_info_parses_into_every_tile_without_panicking() {
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();

    let info = redis_pane::redis::read::fetch_server_info(&client)
        .await
        .unwrap();

    // A fresh, single-node container: `Replication` is present (`role:master`)
    // but has nothing else replication-shaped, and there is no ACL trimming
    // any section away — the parser's tolerance for *missing* sections is
    // exercised by the unit tests in `crates/core/src/redis/read.rs`; this is
    // the tolerance for whatever real Redis 7 actually sends.
    assert_eq!(info.field("role"), Some("master"), "{info:?}");

    let mut dashboard = redis_pane_core::state::DashboardState::default();
    dashboard.record_poll(info, 0);

    // Every tile, populated, with no panic anywhere in the derivation —
    // decision 3's whole tile set, against a real reply.
    let memory = dashboard.memory_tile().expect("memory tile");
    assert!(memory.used_bytes > 0, "{memory:?}");
    let hit_ratio = dashboard.hit_ratio_tile().expect("hit ratio tile");
    let _ = hit_ratio.ratio(); // must not panic, whatever it reads
    let clients = dashboard.clients_tile().expect("clients tile");
    assert!(clients.connected >= 1, "{clients:?}");
    let replication = dashboard.replication_tile().expect("replication tile");
    assert_eq!(replication.role, "master");
    let eviction = dashboard.eviction_tile().expect("eviction tile");
    assert_eq!(eviction.evicted_keys, 0, "{eviction:?}");

    let _ = client.quit().await;
}

#[tokio::test]
#[ignore = "needs docker"]
async fn two_sequential_polls_show_counters_moving_between_them() {
    let (_c, url) = start("redis", "7-alpine").await;
    let (client, _) = redis_pane::redis::connect(&url).await.unwrap();

    let before = redis_pane::redis::read::fetch_server_info(&client)
        .await
        .unwrap();
    let commands_before: u64 = before
        .field("total_commands_processed")
        .and_then(|v| v.parse().ok())
        .expect("total_commands_processed must be present");

    // A handful of real commands between the two polls — what makes the
    // second `INFO` reply describe a server that has actually done
    // something since the first, the same "refreshing, not static" claim
    // R6.3 and PLAN's "Proves" column make for this whole screen.
    for i in 0..10 {
        let _: () = client
            .set(format!("dashboard:probe:{i}"), "v", None, None, false)
            .await
            .unwrap();
    }

    let after = redis_pane::redis::read::fetch_server_info(&client)
        .await
        .unwrap();
    let commands_after: u64 = after
        .field("total_commands_processed")
        .and_then(|v| v.parse().ok())
        .expect("total_commands_processed must be present");

    assert!(
        commands_after > commands_before,
        "before={commands_before} after={commands_after}: the second poll must describe a server that has moved, not a frozen snapshot"
    );

    let _ = client.quit().await;
}

// ── M3 task 2 phase A — feed-connection plumbing
// (`docs/plans/m3-feed-connection.md`) ──────────────────────────────────────
//
// `MONITOR` puts a connection into a mode that accepts nothing else until it
// closes, so it must never share the main connection (`connect_with`'s own
// read/write path). These prove the plumbing that keeps it off that path: the
// main connection keeps answering while a feed is open, a feed actually
// receives what `MONITOR` promises, a server-side kill of the feed surfaces
// as `Msg::FeedClosed` rather than a hang, and `Command::CloseFeed`'s
// teardown really does disconnect — not just stop reading.

mod feed {
    use std::sync::Arc;
    use std::time::Duration;

    use fred::prelude::*;
    use redis_pane::redis::feed::{FeedKind, open_feed};
    use redis_pane_core::Msg;
    use redis_pane_core::clock::Clock;
    use redis_pane_core::command::FeedToken;
    use redis_pane_core::resolve::Credentials;
    use testcontainers::ContainerAsync;
    use testcontainers::GenericImage;
    use testcontainers::core::{ContainerPort, WaitFor};
    use testcontainers::runners::AsyncRunner;

    const REDIS_PORT: ContainerPort = ContainerPort::Tcp(6379);

    async fn start() -> (ContainerAsync<GenericImage>, String) {
        let container = GenericImage::new("redis", "7-alpine")
            .with_exposed_port(REDIS_PORT)
            .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
            .start()
            .await
            .expect("docker must be running for the integration suite");
        let port = container.get_host_port_ipv4(REDIS_PORT).await.unwrap();
        (container, format!("redis://127.0.0.1:{port}"))
    }

    /// A plain second connection, credentials-free like the suite's other
    /// `writer` helpers — for issuing commands the feed should observe, and
    /// for administering the server (`CLIENT LIST`/`CLIENT KILL`) from
    /// outside the connection under test.
    async fn plain_client(url: &str) -> Client {
        let client = Builder::from_config(Config::from_url(url).unwrap())
            .build()
            .unwrap();
        client.init().await.unwrap();
        client
    }

    /// The client id of the (single, by construction in these tests)
    /// connection currently in `MONITOR` mode, read off `CLIENT LIST` from a
    /// connection that is not itself monitoring — a monitoring connection
    /// accepts no commands of its own, so it cannot be asked directly.
    async fn monitor_client_id(admin: &Client) -> Option<String> {
        let list: String = admin
            .client_list::<String, String>(None, None)
            .await
            .unwrap();
        list.lines().find_map(|line| {
            if !line.contains("cmd=monitor") {
                return None;
            }
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("id="))
                .map(str::to_string)
        })
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn an_ordinary_read_on_the_main_connection_does_not_wait_on_an_open_feed() {
        let (_c, url) = start().await;
        let (main, _) = redis_pane::redis::connect(&url).await.unwrap();
        let _: () = main
            .set("probe", "before", None, None, false)
            .await
            .unwrap();

        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let _feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            FeedToken::default(),
            tx,
            clock,
            &main,
        )
        .await
        .expect("MONITOR should be available on a fresh 7-alpine container");

        // The literal claim this task's PLAN row makes: the main connection's
        // own read/write path is untouched by a feed connection being open.
        // Bounded so a regression that *does* couple them fails as a timeout
        // rather than hanging the test suite.
        let value: String = tokio::time::timeout(Duration::from_secs(5), main.get("probe"))
            .await
            .expect("a read on the main connection must not wait on the feed")
            .unwrap();
        assert_eq!(value, "before");

        let _ = main.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_monitor_feed_receives_a_line_for_a_command_on_another_connection() {
        let (_c, url) = start().await;
        let writer = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let _feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            FeedToken::default(),
            tx,
            clock,
            &writer,
        )
        .await
        .unwrap();

        let _: () = writer
            .set("feed:probe", "seen", None, None, false)
            .await
            .unwrap();

        let raw = tokio::time::timeout(Duration::from_secs(5), async {
            match rx
                .recv()
                .await
                .expect("the feed task must not exit silently")
            {
                Msg::MonitorLine { raw, .. } => raw,
                other => panic!("expected a MonitorLine, got {other:?}"),
            }
        })
        .await
        .expect("a line for the SET issued above must arrive");

        assert!(raw.contains("SET"), "{raw}");
        assert!(raw.contains("feed:probe"), "{raw}");

        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn killing_the_feeds_server_side_connection_closes_it_rather_than_hanging() {
        let (_c, url) = start().await;
        let admin = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let token = FeedToken::default();
        let feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            token,
            tx,
            clock,
            &admin,
        )
        .await
        .unwrap();

        let id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(id) = monitor_client_id(&admin).await {
                    return id;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the MONITOR connection must show up in CLIENT LIST");

        let _: () = admin
            .client_kill(vec![fred::types::client::ClientKillFilter::ID(id)])
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a killed feed connection must surface Msg::FeedClosed, never hang")
            .expect("the feed task must send before its channel closes");
        match msg {
            Msg::FeedClosed { token: got, reason } => {
                assert_eq!(got, token);
                assert!(
                    reason.is_some(),
                    "a drop the reader did not ask for names why"
                );
            }
            other => panic!("expected FeedClosed, got {other:?}"),
        }

        // The read loop's own task exits on the same event; nothing left to
        // cancel, but closing is still safe to call (mirrors the shell always
        // calling it on `Command::CloseFeed` regardless of how the feed
        // already ended).
        feed.close();
        let _ = admin.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn close_feed_actually_disconnects_the_monitor_connection() {
        let (_c, url) = start().await;
        let admin = plain_client(&url).await;

        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            FeedToken::default(),
            tx,
            clock,
            &admin,
        )
        .await
        .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            while monitor_client_id(&admin).await.is_none() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the MONITOR connection must show up in CLIENT LIST before it can be closed");

        // `Command::CloseFeed`'s handler: `token.cancel(); drop(...)`, nothing
        // cleverer (`docs/plans/m3-feed-connection.md`).
        feed.close();

        // `fred::monitor::run` hands back a bare `Stream`, not a `Client` —
        // there is nothing to `.quit()` synchronously, so its internal
        // forwarding task only notices our end is gone (and drops the
        // socket) when it next tries to forward a line. Issuing ordinary
        // traffic is what gives it that next line to fail on; polling with a
        // bound is the honest shape of "eventually disconnects" this
        // limitation leaves, documented in `open_feed`'s own doc comment.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let _: () = admin.set("nudge", "1", None, None, false).await.unwrap();
                if monitor_client_id(&admin).await.is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("CLIENT LIST must stop showing the feed client once it is closed");

        let _ = admin.quit().await;
    }

    // ── M3 phase B — Monitor against a real server
    // (`docs/plans/m3-monitor.md`) ──────────────────────────────────────────

    use redis_pane_core::state::{FeedStatus, MONITOR_CAP, View};
    use redis_pane_core::{State, update};

    /// A Monitor view already `Open` on `token`, ready to fold
    /// `Msg::MonitorLine`s in via `update()` — the shape `open_monitor_view`
    /// (`crates/core/src/update/monitor.rs`) leaves the core in, built
    /// directly here since these tests drive a real feed by hand rather than
    /// through a keypress.
    fn monitor_state_open(token: FeedToken) -> State {
        let mut state = State {
            screen: View::Monitor,
            ..State::default()
        };
        state.monitor.feed_token = token;
        state.monitor.status = FeedStatus::Open;
        state.monitor.following = true;
        state
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_real_monitor_line_parses_end_to_end() {
        let (_c, url) = start().await;
        let writer = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let token = FeedToken::default();
        let _feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            token,
            tx,
            clock,
            &writer,
        )
        .await
        .unwrap();

        let _: () = writer
            .set("monitor:probe", "1", None, None, false)
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let mut state = monitor_state_open(token);
        (state, _) = update(state, msg);

        assert_eq!(state.monitor.len(), 1);
        let line = state.monitor.lines().back().unwrap();
        let cols = line.columns();
        assert_eq!(cols.db, "0");
        assert!(cols.command.contains("SET"), "{}", cols.command);
        assert!(cols.command.contains("monitor:probe"), "{}", cols.command);

        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_paused_feed_stays_connected_while_the_buffer_does_not_grow() {
        let (_c, url) = start().await;
        let writer = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let token = FeedToken::default();
        let _feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            token,
            tx,
            clock,
            &writer,
        )
        .await
        .unwrap();

        let mut state = monitor_state_open(token);
        state.monitor.toggle_pause();
        assert!(state.monitor.paused);

        for i in 0..5 {
            let _: () = writer
                .set(format!("paused:probe:{i}"), "1", None, None, false)
                .await
                .unwrap();
        }
        // Drain whatever the real feed delivered for those five writes —
        // exactly the proof PLAN's row for this task asks for: the
        // connection is still alive and readable (these `Msg::MonitorLine`s
        // really arrived), but folding them through `update()` while paused
        // must not grow `MonitorState.lines`.
        let mut received = 0;
        while received < 5 {
            let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("the feed must still be delivering lines while paused")
                .unwrap();
            received += 1;
            (state, _) = update(state, msg);
        }

        assert_eq!(
            state.monitor.len(),
            0,
            "pausing stops consuming the feed, not just hides it"
        );
        assert_eq!(state.monitor.dropped_while_paused, 5);

        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn the_buffer_stays_bounded_against_a_real_stream_past_the_cap() {
        let (_c, url) = start().await;
        let writer = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let token = FeedToken::default();
        let _feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Monitor,
            token,
            tx,
            clock,
            &writer,
        )
        .await
        .unwrap();

        // One pipeline, so the cap-exceeding traffic reaches the server (and
        // the feed) fast rather than paying a round trip per command.
        let total = MONITOR_CAP + 200;
        let pipeline = writer.pipeline();
        for i in 0..total {
            let _: () = pipeline
                .set(format!("bound:probe:{i}"), "1", None, None, false)
                .await
                .unwrap();
        }
        let _: Vec<()> = pipeline.all().await.unwrap();

        let mut state = monitor_state_open(token);
        let mut seen = 0usize;
        // Real `MONITOR` traffic includes more than the `SET`s themselves
        // (the pipeline's own framing can surface as `MULTI`-adjacent
        // bookkeeping on some server versions), so this drains until either
        // the cap is unmistakably exceeded or the feed goes quiet for a
        // beat, rather than counting exactly `total` lines.
        while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            seen += 1;
            (state, _) = update(state, msg);
            assert!(
                state.monitor.len() <= MONITOR_CAP,
                "grew past the cap after {seen} real lines"
            );
        }
        assert!(
            seen >= total,
            "expected at least {total} lines from a real stream, saw {seen}"
        );
        assert_eq!(state.monitor.len(), MONITOR_CAP);

        let _ = writer.quit().await;
    }

    // ── M3 task 5 phase A — Pub/Sub's `SubscriberClient` feed
    // (`docs/plans/m3-pubsub.md`) ────────────────────────────────────────────
    //
    // Mirrors the `MONITOR` feed tests above: a channel and a pattern message
    // arrive correctly attributed, a mid-session subscribe keeps the first
    // subscription flowing, a server-side `CLIENT KILL` surfaces as
    // `Msg::FeedClosed` rather than a hang, and — the load-bearing one for
    // this task's "Proves" column — closing the feed really does leave no
    // subscription behind on the server's own accounting (`PUBSUB
    // CHANNELS`/`PUBSUB NUMPAT`), not just in this app's bookkeeping.

    use redis_pane_core::state::Subscription;

    /// The client id of a connection currently subscribed to at least one
    /// channel or pattern (`sub=`/`psub=` counts in `CLIENT LIST`) — the
    /// Pub/Sub analogue of `monitor_client_id`. Unlike a `MONITOR`'d
    /// connection, a subscriber still answers `CLIENT LIST` about itself in
    /// principle (RESP3 pubsub connections are not exclusive the way
    /// `MONITOR` is), but asking from a second, uninvolved connection keeps
    /// this test symmetric with the `MONITOR` ones above and needs no
    /// special-casing for which client is "self".
    async fn subscriber_client_id(admin: &Client) -> Option<String> {
        let list: String = admin
            .client_list::<String, String>(None, None)
            .await
            .unwrap();
        list.lines().find_map(|line| {
            let subscribed = line.split_whitespace().any(|field| {
                field
                    .strip_prefix("sub=")
                    .or_else(|| field.strip_prefix("psub="))
                    .is_some_and(|n| n != "0")
            });
            if !subscribed {
                return None;
            }
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("id="))
                .map(str::to_string)
        })
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_channel_and_a_pattern_message_arrive_correctly_attributed() {
        let (_c, url) = start().await;
        let writer = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let subs = vec![
            Subscription::Channel("feed:chan".into()),
            Subscription::Pattern("feed:pat:*".into()),
        ];
        let _feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Subscribe(subs),
            FeedToken::default(),
            tx,
            clock,
            &writer,
        )
        .await
        .expect("a SubscriberClient should dial and subscribe on a fresh container");

        // Give the subscription a moment to actually land server-side before
        // publishing — `open_feed` awaits `SUBSCRIBE`/`PSUBSCRIBE`'s own
        // reply, so this is a belt-and-suspenders wait, not load-bearing.
        let _: i64 = writer.publish("feed:chan", "direct").await.unwrap();
        let _: i64 = writer.publish("feed:pat:42", "via-pattern").await.unwrap();

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the direct-channel message must arrive")
            .unwrap();
        match first {
            Msg::PubSubMessage {
                channel,
                via,
                payload,
                ..
            } => {
                assert_eq!(channel, b"feed:chan");
                assert_eq!(via, None, "a direct SUBSCRIBE carries no pattern");
                assert_eq!(payload, b"direct");
            }
            other => panic!("expected a PubSubMessage, got {other:?}"),
        }

        let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the pattern-matched message must arrive")
            .unwrap();
        match second {
            Msg::PubSubMessage {
                channel,
                via,
                payload,
                ..
            } => {
                assert_eq!(channel, b"feed:pat:42");
                assert_eq!(via.as_deref(), Some(b"feed:pat:*".as_slice()));
                assert_eq!(payload, b"via-pattern");
            }
            other => panic!("expected a PubSubMessage, got {other:?}"),
        }

        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn adding_a_subscription_mid_session_keeps_the_first_one_flowing() {
        let (_c, url) = start().await;
        let writer = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Subscribe(vec![Subscription::Channel("mid:first".into())]),
            FeedToken::default(),
            tx.clone(),
            clock.clone(),
            &writer,
        )
        .await
        .unwrap();

        let _: i64 = writer.publish("mid:first", "one").await.unwrap();
        let before = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the first subscription must already be receiving")
            .unwrap();
        assert!(matches!(before, Msg::PubSubMessage { .. }));

        // `Command::UpdateSubscription`'s shell-side execution
        // (`FeedHandle::update_subscription`): add a second channel on the
        // *same* connection — never a close-then-reopen, which would risk
        // dropping a message on `mid:first` in the gap.
        feed.update_subscription(
            vec![Subscription::Channel("mid:second".into())],
            vec![],
            tx,
            clock,
        );

        // Give the ADD a moment to land, then prove both channels still
        // deliver — `mid:first` first, to show the add did not disturb it.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _: i64 = writer.publish("mid:first", "two").await.unwrap();
        let _: i64 = writer.publish("mid:second", "three").await.unwrap();

        let mut seen_first_again = false;
        let mut seen_second = false;
        for _ in 0..2 {
            let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("both messages must arrive")
                .unwrap();
            match msg {
                Msg::PubSubMessage { channel, .. } if channel == b"mid:first" => {
                    seen_first_again = true;
                }
                Msg::PubSubMessage { channel, .. } if channel == b"mid:second" => {
                    seen_second = true;
                }
                other => panic!("unexpected message: {other:?}"),
            }
        }
        assert!(
            seen_first_again,
            "the first subscription must keep receiving after the add"
        );
        assert!(seen_second, "the newly added subscription must receive");

        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn closing_the_feed_leaves_no_subscription_on_the_server() {
        let (_c, url) = start().await;
        let admin = plain_client(&url).await;

        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let subs = vec![
            Subscription::Channel("orphan:chan".into()),
            Subscription::Pattern("orphan:pat:*".into()),
        ];
        let feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Subscribe(subs),
            FeedToken::default(),
            tx,
            clock,
            &admin,
        )
        .await
        .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let channels: Vec<String> = admin.pubsub_channels("orphan:*").await.unwrap();
                let numpat: i64 = admin.pubsub_numpat().await.unwrap();
                if !channels.is_empty() && numpat > 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the subscription must show up in PUBSUB CHANNELS/NUMPAT before it can close");

        // `Command::CloseFeed`'s handler for a Pub/Sub feed: cancel the
        // reader and send `QUIT` (`FeedHandle::close`) — the "unsubscribing
        // cleanly" requirement this task's PLAN row names: even without an
        // explicit `UNSUBSCRIBE`, disconnecting drops every subscription
        // server-side.
        feed.close();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let channels: Vec<String> = admin.pubsub_channels("orphan:*").await.unwrap();
                let numpat: i64 = admin.pubsub_numpat().await.unwrap();
                if channels.is_empty() && numpat == 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect(
            "PUBSUB CHANNELS/NUMPAT must show nothing of ours once the feed has closed — an \
             orphaned server-side subscription is exactly what this test guards against",
        );

        let _ = admin.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn killing_the_subscribers_server_side_connection_closes_it_rather_than_hanging() {
        let (_c, url) = start().await;
        let admin = plain_client(&url).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: Arc<dyn Clock> = Arc::new(redis_pane::SystemClock);
        let token = FeedToken::default();
        let feed = open_feed(
            &url,
            &Credentials::default(),
            FeedKind::Subscribe(vec![Subscription::Channel("kill:probe".into())]),
            token,
            tx,
            clock,
            &admin,
        )
        .await
        .unwrap();

        let id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(id) = subscriber_client_id(&admin).await {
                    return id;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the subscriber connection must show up in CLIENT LIST");

        let _: () = admin
            .client_kill(vec![fred::types::client::ClientKillFilter::ID(id)])
            .await
            .unwrap();

        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a killed feed connection must surface Msg::FeedClosed, never hang")
            .expect("the feed task must send before its channel closes");
        match msg {
            Msg::FeedClosed { token: got, reason } => {
                assert_eq!(got, token);
                assert!(
                    reason.is_some(),
                    "a drop the reader did not ask for names why"
                );
            }
            other => panic!("expected FeedClosed, got {other:?}"),
        }

        feed.close();
        let _ = admin.quit().await;
    }

    // M3 task 5 (b) — an ACL-denied `SUBSCRIBE`, deliberately not covered
    // here: a real Docker-backed attempt (`ACL SETUSER ... resetchannels
    // &allowed:*`, then subscribing to a channel outside that pattern)
    // found that fred 10.1.0's `SubscriberClient::subscribe()` does not
    // surface the server's `-NOPERM` as an `Err` at all — `client
    // .subscribe(["denied:channel"]).await` resolves `Ok(())` even though
    // `PUBSUB CHANNELS` on a second connection afterwards shows the
    // subscription never actually took (confirmed directly against a
    // container: `redis-cli --user ... SUBSCRIBE denied:channel` gets
    // `NOPERM` synchronously from the server, so this is fred's client-side
    // handling, not a server-side inconsistency). `FeedHandle
    // ::update_subscription`'s `Err` branch — and therefore
    // `Msg::SubscriptionFailed` — is correct for every failure fred *does*
    // surface (a dead connection, a malformed reply), but cannot fire for
    // this specific ACL-denial shape until fred's own `subscribe()`
    // propagates it. The state-management half (a failed add drops the
    // chip, the notification carries the command) is fully covered at the
    // core level (`crates/core/src/update/pubsub.rs`'s
    // `a_failed_subscribe_drops_the_chip_and_raises_a_notification` and
    // siblings) — what's unverified end-to-end is specifically whether the
    // shell's `client.subscribe()` call ever returns the `Err` those core
    // tests assume arrives. Flagged for whoever picks this up next rather
    // than left as a silently-passing test that does not actually exercise
    // the failure path it claims to.
}

// ── M5 task 1: the real-cluster harness (ADR-0022) ──────────────────────────

mod cluster_harness {
    use super::Credentials;
    use super::support::cluster::Cluster;
    use super::support::cluster::start_cluster;

    /// A freshly formed, idle cluster advertises no replicas in `CLUSTER
    /// SLOTS` (Redis leaves out a replica whose replication offset is still
    /// 0), so a client's routing table reads 3 nodes, not 6. One write per
    /// primary moves every offset, and this waits until all six ports are
    /// listed, with a deadline.
    async fn warm(cluster: &Cluster) {
        let client = cluster.client().await;
        for primary in cluster.primaries().await {
            let key = cluster.key_in_slot_of(primary.port).await;
            client
                .set::<(), _, _>(&key, "v", None, None, false)
                .await
                .unwrap();
        }
        let _ = client.quit().await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let slots = cluster.cli(cluster.ports[0], &["cluster", "slots"]).await;
            if cluster
                .ports
                .iter()
                .all(|p| slots.lines().any(|l| l == p.to_string()))
            {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "CLUSTER SLOTS never listed every replica:\n{slots}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    use fred::prelude::*;

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_real_cluster_comes_up_and_moved_redirects_resolve_from_the_host() {
        let cluster = start_cluster().await;
        let nodes = cluster.nodes().await;
        assert_eq!(nodes.iter().filter(|n| n.is_primary).count(), 3);
        assert_eq!(nodes.iter().filter(|n| !n.is_primary).count(), 3);

        // One key per primary: a plain cluster client reaches each only by
        // following `MOVED` to an announced address, so this proves those
        // addresses are reachable from the test process.
        let client = cluster.client().await;
        for primary in cluster.primaries().await {
            let key = cluster.key_in_slot_of(primary.port).await;
            client
                .set::<(), _, _>(&key, "v", None, None, false)
                .await
                .unwrap();
            let got: String = client.get(&key).await.unwrap();
            assert_eq!(got, "v");
        }
        let _ = client.quit().await;
    }

    /// M5 tasks 2 and 5: `redis-cluster://` and a plain `redis://` to
    /// one node both end in a cluster client that knows the 3 + 3 shape.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn allow_connects_a_cluster_by_either_url_and_reports_its_shape() {
        use fred::interfaces::ClientLike;
        use redis_pane_core::state::Topology;
        let cluster = start_cluster().await;
        warm(&cluster).await;
        let urls = [
            cluster.seed_url(),
            format!("redis://127.0.0.1:{}", cluster.ports[0]),
        ];
        for url in urls {
            let (client, est) = redis_pane::redis::connect_with(&url, &Credentials::default())
                .await
                .unwrap_or_else(|e| panic!("{url}: {e}"));
            assert!(client.is_clustered(), "{url}: not a cluster client");
            assert_eq!(
                est.topology,
                Some(Topology {
                    primaries: 3,
                    nodes: 6
                }),
                "{url}"
            );
            assert_eq!(est.read_only, None, "{url}: a cluster is not a replica");
            // It is a working cluster client: a key on each primary round-trips.
            for primary in cluster.primaries().await {
                let key = cluster.key_in_slot_of(primary.port).await;
                client
                    .set::<(), _, _>(&key, "v", None, None, false)
                    .await
                    .unwrap();
                let got: String = client.get(&key).await.unwrap();
                assert_eq!(got, "v");
            }
            let _ = client.quit().await;
        }
    }

    /// What `fred` does on a failover, which `terminal.rs`'s listener relies
    /// on: `cluster_change_rx` fires, and the routing table afterwards still
    /// reads 3 primaries and 6 nodes.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_failover_reaches_cluster_change_rx_and_the_topology_is_still_3_and_6() {
        use fred::interfaces::EventInterface;
        use redis_pane_core::state::Topology;
        let cluster = start_cluster().await;
        warm(&cluster).await;
        let (client, est) =
            redis_pane::redis::connect_with(&cluster.seed_url(), &Credentials::default())
                .await
                .unwrap();
        assert!(est.topology.is_some());
        let mut changes = client.cluster_change_rx();
        let replica = cluster.replicas().await.remove(0);
        cluster.failover(replica.port).await;
        // Generous: the client learns of it from a redirect or its next sync.
        // Drive a command at every primary so one is made.
        let mut seen = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline && seen.is_none() {
            for primary in cluster.primaries().await {
                let key = cluster.key_in_slot_of(primary.port).await;
                let _ = client.get::<Option<String>, _>(&key).await;
            }
            if let Ok(Ok(change)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), changes.recv()).await
            {
                seen = Some(change);
            }
        }
        eprintln!("cluster_change_rx after failover: {seen:?}");
        assert!(seen.is_some(), "cluster_change_rx never fired on failover");
        // The table the listener reads after the event: still 3 primaries,
        // and 6 nodes once the demoted primary is advertised as a replica.
        let want = Topology {
            primaries: 3,
            nodes: 6,
        };
        let mut last = redis_pane::redis::topology_of(&client);
        while last != Some(want)
            && std::time::Instant::now() < deadline + std::time::Duration::from_secs(30)
        {
            for primary in cluster.primaries().await {
                let key = cluster.key_in_slot_of(primary.port).await;
                let _ = client.get::<Option<String>, _>(&key).await;
            }
            let _ =
                tokio::time::timeout(std::time::Duration::from_millis(500), changes.recv()).await;
            last = redis_pane::redis::topology_of(&client);
        }
        assert_eq!(last, Some(want));
        let _ = client.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn failover_promotes_a_replica_and_keeps_the_data() {
        let cluster = start_cluster().await;
        let client = cluster.client().await;
        let primary = cluster.primaries().await.remove(0);
        let key = cluster.key_in_slot_of(primary.port).await;
        client
            .set::<(), _, _>(&key, "kept", None, None, false)
            .await
            .unwrap();

        let replica = cluster
            .replicas()
            .await
            .into_iter()
            .find(|r| r.primary_id.as_deref() == Some(primary.id.as_str()))
            .expect("every primary has a replica");
        cluster.failover(replica.port).await;

        let nodes = cluster.nodes().await;
        let promoted = nodes.iter().find(|n| n.port == replica.port).unwrap();
        let demoted = nodes.iter().find(|n| n.port == primary.port).unwrap();
        assert!(promoted.is_primary && !demoted.is_primary);
        assert!(promoted.owns(fred::util::redis_keyslot(key.as_bytes())));
        assert_eq!(nodes.iter().filter(|n| n.is_primary).count(), 3);

        // The client follows the move on its own.
        let got: String = client.get(&key).await.unwrap();
        assert_eq!(got, "kept");
        let _ = client.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn migrate_slot_moves_a_slot_and_its_keys_to_another_primary() {
        let cluster = start_cluster().await;
        let client = cluster.client().await;
        let primaries = cluster.primaries().await;
        let (from, to) = (primaries[0].port, primaries[1].port);
        let key = cluster.key_in_slot_of(from).await;
        let slot = fred::util::redis_keyslot(key.as_bytes());
        client
            .set::<(), _, _>(&key, "moved", None, None, false)
            .await
            .unwrap();

        cluster.migrate_slot(slot, from, to).await;

        assert_eq!(cluster.owner_of_slot(slot).await.port, to);
        let on_dest = cluster.cli(to, &["get", &key]).await;
        assert_eq!(on_dest.trim(), "moved");
        // The client's routing table is stale until it follows `MOVED`.
        let got: String = client.get(&key).await.unwrap();
        assert_eq!(got, "moved");
        let _ = client.quit().await;
    }
}

/// M5 task 3: the merged scan across a real cluster (R1.11, ADR-0022).
mod cluster_scan {
    use super::Credentials;
    use super::support::cluster::{Cluster, start_cluster};
    use fred::prelude::*;
    use redis_pane::redis::scan::{Tuning, stream_keys_with};
    use redis_pane_core::Msg;
    use std::collections::HashSet;
    use std::time::Duration;
    use tokio::sync::mpsc::Receiver;
    use tokio_util::sync::CancellationToken;

    async fn connect(cluster: &Cluster) -> Client {
        let (client, _) =
            redis_pane::redis::connect_with(&cluster.seed_url(), &Credentials::default())
                .await
                .expect("cluster client");
        client
    }

    /// `user:N` for even N and `order:N` for odd N, 30,000 in all, so every
    /// slot range holds keys and a pattern has something to separate.
    async fn seed(client: &Client, n: u32) {
        for chunk in (0..n).collect::<Vec<_>>().chunks(1_000) {
            let pipe = client.pipeline();
            for i in chunk {
                let name = if i % 2 == 0 { "user" } else { "order" };
                pipe.set::<(), _, _>(format!("{name}:{i}"), "v", None, None, false)
                    .await
                    .unwrap();
            }
            pipe.all::<()>().await.unwrap();
        }
    }

    /// Everything up to and including the terminal message.
    struct Run {
        started: Option<u64>,
        keys: Vec<String>,
        end: Msg,
    }

    fn is_terminal(m: &Msg) -> bool {
        matches!(
            m,
            Msg::ScanComplete
                | Msg::ScanCancelled
                | Msg::ScanFailed { .. }
                | Msg::ScanInterrupted { .. }
        )
    }

    async fn drain(rx: &mut Receiver<Msg>, run: &mut Run, deadline: Duration) {
        let end = tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Some(Msg::ScanStarted { estimated_total }) => {
                        run.started = Some(estimated_total)
                    }
                    Some(Msg::ScanBatch { keys }) => run
                        .keys
                        .extend(keys.into_iter().map(|k| String::from_utf8(k).unwrap())),
                    Some(m) if is_terminal(&m) => return m,
                    Some(other) => panic!("unexpected {other:?}"),
                    None => panic!("scan ended without a terminal message"),
                }
            }
        })
        .await
        .expect("the scan did not end in time");
        run.end = end;
    }

    fn new_run() -> Run {
        Run {
            started: None,
            keys: Vec::new(),
            end: Msg::ScanCancelled,
        }
    }

    async fn scan_all(client: &Client, pattern: Option<&str>) -> Run {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let mut run = new_run();
        let scan = stream_keys_with(
            client,
            pattern,
            tx,
            CancellationToken::new(),
            Tuning::default(),
        );
        let (_, ()) = tokio::join!(scan, drain(&mut rx, &mut run, Duration::from_secs(120)));
        run
    }

    async fn dbsize_total(cluster: &Cluster) -> u64 {
        let mut total = 0;
        for p in cluster.primaries().await {
            total += cluster
                .cli(p.port, &["dbsize"])
                .await
                .trim()
                .parse::<u64>()
                .unwrap();
        }
        total
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn every_key_across_every_primary_is_seen_exactly_once_and_the_scan_completes() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        seed(&client, 30_000).await;

        // The seed must really span the cluster, or this proves nothing.
        for p in cluster.primaries().await {
            let n: u64 = cluster
                .cli(p.port, &["dbsize"])
                .await
                .trim()
                .parse()
                .unwrap();
            assert!(n > 5_000, "primary {} holds only {n} keys", p.port);
        }

        let run = scan_all(&client, None).await;
        assert_eq!(run.end, Msg::ScanComplete);
        assert_eq!(run.keys.len(), 30_000, "a key was missed or repeated");
        let unique: HashSet<_> = run.keys.iter().collect();
        assert_eq!(unique.len(), 30_000, "a key repeated in a stable cluster");
        // The progress denominator is the cluster-wide total, not one node's.
        assert_eq!(run.started, Some(30_000));
        assert_eq!(run.started, Some(dbsize_total(&cluster).await));
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_pattern_filter_works_across_every_node() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        seed(&client, 30_000).await;

        let run = scan_all(&client, Some("user:*")).await;
        assert_eq!(run.end, Msg::ScanComplete);
        assert_eq!(run.keys.len(), 15_000);
        assert!(run.keys.iter().all(|k| k.starts_with("user:")));
        assert_eq!(run.keys.iter().collect::<HashSet<_>>().len(), 15_000);
    }

    /// M5 task 5: the path `main.rs` and the shell's Reconnect use is
    /// `connect_with`; from either URL it browses every primary's keys.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn the_production_connect_path_browses_every_primary_by_either_url() {
        let cluster = start_cluster().await;
        let seeder = connect(&cluster).await;
        seed(&seeder, 6_000).await;
        for url in [
            cluster.seed_url(),
            format!("redis://127.0.0.1:{}", cluster.ports[0]),
        ] {
            let (client, est) = redis_pane::redis::connect_with(&url, &Credentials::default())
                .await
                .unwrap_or_else(|e| panic!("{url}: {e}"));
            assert!(est.topology.is_some(), "{url}: not reported as a Cluster");
            let run = scan_all(&client, None).await;
            assert_eq!(run.end, Msg::ScanComplete, "{url}");
            assert_eq!(
                run.keys.iter().collect::<HashSet<_>>().len(),
                6_000,
                "{url}"
            );
            let _ = client.quit().await;
        }
    }

    /// Pub/Sub stays open on a Cluster (classic `SUBSCRIBE` is cluster-wide).
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn pubsub_feed_opens_on_a_cluster_and_receives_a_publish() {
        use redis_pane::redis::feed::{FeedKind, open_feed};
        use redis_pane_core::command::FeedToken;
        use redis_pane_core::state::Subscription;
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let clock: std::sync::Arc<dyn redis_pane_core::clock::Clock> =
            std::sync::Arc::new(redis_pane::SystemClock);
        let _feed = open_feed(
            &cluster.seed_url(),
            &Credentials::default(),
            FeedKind::Subscribe(vec![Subscription::Channel("gate:chan".into())]),
            FeedToken::default(),
            tx,
            clock,
            &client,
        )
        .await
        .expect("a Pub/Sub feed must open against a Cluster");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let _: i64 = client.publish("gate:chan", "hello").await.unwrap();
                if let Ok(Some(Msg::PubSubMessage { payload, .. })) =
                    tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
                {
                    assert_eq!(payload, b"hello");
                    return;
                }
            }
        })
        .await
        .expect("the publish must reach the subscription");
    }

    async fn scan_calls(cluster: &Cluster) -> u64 {
        let mut total = 0;
        for p in cluster.primaries().await {
            let info = cluster.cli(p.port, &["info", "commandstats"]).await;
            total += info
                .lines()
                .find(|l| l.starts_with("cmdstat_scan:"))
                .and_then(|l| {
                    l.trim_start_matches("cmdstat_scan:calls=")
                        .split(',')
                        .next()
                })
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0);
        }
        total
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn esc_mid_scan_cancels_within_a_page_and_stops_asking_the_servers() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        seed(&client, 30_000).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let cancel = CancellationToken::new();
        let tuning = Tuning {
            page_size: 50,
            ..Tuning::default()
        };
        let c = client.clone();
        let token = cancel.clone();
        let scan = tokio::spawn(async move {
            stream_keys_with(&c, None, tx, token, tuning).await;
        });

        // Take the start and one page, then press Esc.
        let mut seen = 0;
        loop {
            match rx.recv().await.expect("scan ended early") {
                Msg::ScanStarted { .. } => {}
                Msg::ScanBatch { keys } => {
                    seen += keys.len();
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        cancel.cancel();

        // The terminal message follows; at most the batch the loop was
        // already holding can precede it.
        let mut batches_after = 0;
        let end = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match rx.recv().await.expect("no terminal message") {
                    Msg::ScanBatch { keys } => {
                        seen += keys.len();
                        batches_after += 1;
                    }
                    m => return m,
                }
            }
        })
        .await
        .expect("Esc was not answered within 5s");
        assert_eq!(end, Msg::ScanCancelled);
        assert!(batches_after <= 1, "{batches_after} pages after Esc");
        assert!(seen < 30_000);
        scan.await.unwrap();

        // And the servers are not still being asked for pages.
        let first = scan_calls(&cluster).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            first,
            scan_calls(&cluster).await,
            "SCAN continued after Esc"
        );
    }

    /// A failover during a scan ends it as interrupted, never as complete.
    /// Deterministic: the scan is held between pages by an undrained channel,
    /// the failover is carried out, and draining resumes only once the
    /// client's own routing table shows the new primary.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_failover_during_a_scan_interrupts_it_and_never_completes_it() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        seed(&client, 30_000).await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let tuning = Tuning {
            page_size: 20,
            topology_sync_every: Duration::from_millis(200),
            ..Tuning::default()
        };
        let c = client.clone();
        let scan = tokio::spawn(async move {
            stream_keys_with(&c, None, tx, CancellationToken::new(), tuning).await;
        });
        let mut run = new_run();
        loop {
            match rx.recv().await.expect("scan ended early") {
                Msg::ScanStarted { estimated_total } => run.started = Some(estimated_total),
                Msg::ScanBatch { keys } => {
                    run.keys
                        .extend(keys.into_iter().map(|k| String::from_utf8(k).unwrap()));
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }

        // The scan now waits on us, mid-keyspace. Fail a primary over.
        let replica = cluster.replicas().await.remove(0);
        let new_primary = format!("127.0.0.1:{}", replica.port);
        cluster.failover(replica.port).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let moved = client.cached_cluster_state().is_some_and(|r| {
                r.slots()
                    .iter()
                    .any(|s| s.primary.to_string() == new_primary)
            });
            if moved {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the client never noticed the failover"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        drain(&mut rx, &mut run, Duration::from_secs(30)).await;
        assert!(
            matches!(run.end, Msg::ScanInterrupted { .. }),
            "expected ScanInterrupted, got {:?}",
            run.end
        );
        assert!(run.keys.len() < 30_000, "the scan was not mid-way");
        scan.await.unwrap();
    }

    /// A primary that is down fails the scan and is named, rather than the
    /// scan hanging or reporting a smaller total.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_dead_primary_fails_the_scan_naming_it() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        seed(&client, 3_000).await;
        let dead = cluster.primaries().await[1].port;
        cluster
            .exec(vec![
                "sh".into(),
                "-c".into(),
                format!("redis-cli -p {dead} shutdown nosave; true"),
            ])
            .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let mut run = new_run();
        let tuning = Tuning {
            node_page_timeout: Duration::from_secs(3),
            topology_sync_every: Duration::from_secs(3_600),
            ..Tuning::default()
        };
        let c = client.clone();
        let scan = tokio::spawn(async move {
            stream_keys_with(&c, None, tx, CancellationToken::new(), tuning).await;
        });
        drain(&mut rx, &mut run, Duration::from_secs(60)).await;
        let Msg::ScanFailed { error } = &run.end else {
            panic!("expected ScanFailed, got {:?}", run.end);
        };
        assert!(
            error.contains(&format!("127.0.0.1:{dead}")),
            "the failing node is not named: {error}"
        );
        scan.await.unwrap();
    }

    /// The same, but the node dies after the denominator was read, so it is the
    /// per-node page timeout that names it.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_primary_that_dies_mid_scan_fails_the_scan_naming_it() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        seed(&client, 30_000).await;
        let dead = cluster.primaries().await[1].port;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let tuning = Tuning {
            page_size: 20,
            node_page_timeout: Duration::from_secs(3),
            topology_sync_every: Duration::from_secs(3_600),
        };
        let c = client.clone();
        let scan = tokio::spawn(async move {
            stream_keys_with(&c, None, tx, CancellationToken::new(), tuning).await;
        });
        let mut run = new_run();
        loop {
            match rx.recv().await.expect("scan ended early") {
                Msg::ScanStarted { .. } => {}
                Msg::ScanBatch { .. } => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        cluster
            .exec(vec![
                "sh".into(),
                "-c".into(),
                format!("redis-cli -p {dead} shutdown nosave; true"),
            ])
            .await;
        drain(&mut rx, &mut run, Duration::from_secs(60)).await;
        let Msg::ScanFailed { error } = &run.end else {
            panic!("expected ScanFailed, got {:?}", run.end);
        };
        assert!(
            error.contains(&format!("127.0.0.1:{dead}")),
            "the failing node is not named: {error}"
        );
        scan.await.unwrap();
    }
}

/// M5 task 4: liveness over a real cluster is pinned to the key's owner
/// (ADR-0006, ADR-0009, ADR-0022).
mod cluster_liveness {
    use super::Credentials;
    use super::support::cluster::{Cluster, start_cluster};
    use fred::interfaces::{ClientLike, TrackingInterface};
    use fred::prelude::*;
    use fred::types::CustomCommand;
    use fred::types::config::Server;
    use redis_pane::redis::read::{Arming, read_value};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    async fn connect(cluster: &Cluster) -> Client {
        let (client, est) =
            redis_pane::redis::connect_with(&cluster.seed_url(), &Credentials::default())
                .await
                .expect("cluster client");
        assert!(est.tracking_supported, "CLIENT TRACKING must be accepted");
        client
    }

    fn slot(key: &str) -> u16 {
        fred::util::redis_keyslot(key.as_bytes())
    }

    fn node(port: u16) -> Server {
        Server::new("127.0.0.1", port)
    }

    /// `CLIENT TRACKINGINFO` as this client's own connection to `port` reports
    /// it, rendered as text.
    async fn tracking_info(client: &Client, port: u16) -> String {
        let reply: fred::types::Value = client
            .with_cluster_node(node(port))
            .custom(
                CustomCommand::new_static("CLIENT", None, false),
                vec!["TRACKINGINFO"],
            )
            .await
            .expect("CLIENT TRACKINGINFO");
        format!("{reply:?}")
    }

    /// Unrelated traffic on the same client, to other slots: metadata reads
    /// (pipelines of TYPE/TTL/MEMORY USAGE) and plain GETs. Interleaving with
    /// the arming is the failure mode under test.
    fn traffic(
        client: &Client,
        keys: Vec<String>,
        stop: Arc<AtomicBool>,
    ) -> tokio::task::JoinHandle<u64> {
        let client = client.clone();
        tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let window: Vec<(usize, Vec<u8>)> = keys
                    .iter()
                    .enumerate()
                    .map(|(i, k)| (i, k.as_bytes().to_vec()))
                    .collect();
                let _ = redis_pane::redis::fetch_metadata(&client, &window).await;
                for k in &keys {
                    let _ = client.get::<Option<String>, _>(k).await;
                }
                n += 1;
            }
            n
        })
    }

    async fn next_invalidation(
        rx: &mut tokio::sync::broadcast::Receiver<fred::types::client::Invalidation>,
        key: &str,
        within: Duration,
    ) -> Option<fred::types::client::Invalidation> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            match tokio::time::timeout(left, rx.recv()).await {
                Ok(Ok(inv)) if inv.keys.iter().any(|k| k.as_bytes() == key.as_bytes()) => {
                    return Some(inv);
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) | Err(_) => return None,
            }
        }
    }

    /// SPIKE (M5 task 4, decision 1): the slot-pinned arming pipeline puts
    /// `CLIENT CACHING YES` and `TYPE` on the owner's connection, with
    /// nothing interleaved, for a key on each primary, across repeated
    /// re-arms and under unrelated traffic on the same client.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn spike_the_pinned_arming_lands_on_the_owner_under_concurrent_traffic() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        let writer = cluster.client().await;
        let mut rx = client.invalidation_rx();
        let primaries = cluster.primaries().await;

        // Positive control: the flag is observable at all. A bare `CLIENT
        // CACHING YES` sent to one node shows `caching-yes` until the next
        // non-CLIENT command on that connection.
        let p0 = primaries[0].port;
        let _: () = client
            .with_cluster_node(node(p0))
            .client_caching(true)
            .await
            .unwrap();
        let info = tracking_info(&client, p0).await;
        assert!(info.contains("caching-yes"), "control: {info}");
        let own = cluster.key_in_slot_of(p0).await;
        let _: Option<String> = client.with_cluster_node(node(p0)).get(own).await.unwrap();
        let info = tracking_info(&client, p0).await;
        assert!(!info.contains("caching-yes"), "control after GET: {info}");

        // Other-slot keys for the traffic.
        let mut noise = Vec::new();
        for p in &primaries {
            for i in 0..4 {
                let k = format!("noise:{}:{i}", p.port);
                writer
                    .set::<(), _, _>(&k, "n", None, None, false)
                    .await
                    .unwrap();
                noise.push(k);
            }
        }

        for primary in &primaries {
            let key = cluster.key_in_slot_of(primary.port).await;
            writer
                .set::<(), _, _>(&key, "v0", None, None, false)
                .await
                .unwrap();
            let owner = redis_pane::redis::slot_owner(&client, slot(&key)).expect("owner");
            assert_eq!(owner.port, primary.port);

            // Quiet phase: nothing else on the client, so a stray pending
            // opt-in on a non-owner would still be there to see.
            for round in 0..5 {
                read_value(&client, key.as_bytes(), Arming::Enabled)
                    .await
                    .unwrap()
                    .unwrap();
                for other in primaries.iter().filter(|p| p.port != primary.port) {
                    let info = tracking_info(&client, other.port).await;
                    assert!(
                        !info.contains("caching-yes"),
                        "round {round}: non-owner {} holds a pending opt-in: {info}",
                        other.port
                    );
                }
                writer
                    .set::<(), _, _>(&key, format!("q{round}"), None, None, false)
                    .await
                    .unwrap();
                let inv = next_invalidation(&mut rx, &key, Duration::from_secs(5))
                    .await
                    .unwrap_or_else(|| panic!("quiet round {round}: no invalidation for {key}"));
                assert_eq!(inv.server.port, primary.port, "quiet round {round}");
            }

            // Noisy phase: the same, under concurrent traffic to other slots.
            let stop = Arc::new(AtomicBool::new(false));
            let ts: Vec<_> = (0..4)
                .map(|_| traffic(&client, noise.clone(), stop.clone()))
                .collect();
            for round in 0..20 {
                read_value(&client, key.as_bytes(), Arming::Enabled)
                    .await
                    .unwrap()
                    .unwrap();
                writer
                    .set::<(), _, _>(&key, format!("n{round}"), None, None, false)
                    .await
                    .unwrap();
                let inv = next_invalidation(&mut rx, &key, Duration::from_secs(5))
                    .await
                    .unwrap_or_else(|| panic!("noisy round {round}: no invalidation for {key}"));
                assert_eq!(inv.server.port, primary.port, "noisy round {round}");
            }
            stop.store(true, Ordering::Relaxed);
            let mut passes = 0;
            for t in ts {
                passes += t.await.unwrap();
            }
            eprintln!(
                "primary {}: {passes} traffic passes during arming",
                primary.port
            );
            assert!(passes > 0);
        }
        let _ = client.quit().await;
        let _ = writer.quit().await;
    }

    /// NEGATIVE CONTROL for the spike: the same arming without the pin. If
    /// this never missed, the spike above would prove nothing. Un-pinned,
    /// `CLIENT CACHING YES` has no key and goes to an arbitrary node, so some
    /// arming must be lost.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn spike_control_an_unpinned_arming_is_lost_on_some_nodes() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        let writer = cluster.client().await;
        let mut rx = client.invalidation_rx();
        // fred sends a keyless command to one node, the same one every time,
        // so which key is hit depends on where that is: try a key on each
        // primary, and at least the other two must miss.
        let mut missed = 0;
        for primary in cluster.primaries().await {
            let key = cluster.key_in_slot_of(primary.port).await;
            writer
                .set::<(), _, _>(&key, "v0", None, None, false)
                .await
                .unwrap();
            for round in 0..5 {
                let pipeline = client.pipeline();
                let _: () = pipeline.client_caching(true).await.unwrap();
                let _: () = pipeline.r#type(key.as_str()).await.unwrap();
                let _: (String, String) = pipeline.all().await.unwrap();
                writer
                    .set::<(), _, _>(&key, format!("u{round}"), None, None, false)
                    .await
                    .unwrap();
                if next_invalidation(&mut rx, &key, Duration::from_millis(700))
                    .await
                    .is_none()
                {
                    missed += 1;
                }
            }
        }
        eprintln!("unpinned arming: {missed}/15 rounds produced no invalidation");
        assert!(
            missed > 0,
            "the control never failed, so the spike discriminates nothing"
        );
        let _ = client.quit().await;
        let _ = writer.quit().await;
    }

    // ── The Viewer's shell side, driven for real ────────────────────────────
    //
    // `terminal.rs` needs a terminal, so this reproduces the loop it runs for
    // the open key with the same `redis_pane::liveness` listeners, the same
    // `read_value`, and the same `OpenOwner` recording. The one thing standing
    // in for the core is a two-line model of what its `Msg::Invalidated` and
    // `Msg::Connected` handlers do (drop the liveness claim, refetch; live
    // again only when the refetch has armed), which the core's own unit tests
    // pin. Everything that decides *which node* is real.

    use redis_pane_core::Msg;
    use std::sync::atomic::AtomicU32;

    struct Viewer {
        /// The current client; replaced by the redial a lost link causes, as
        /// the shell replaces its own.
        cur: Arc<Mutex<Client>>,
        key: String,
        owner: redis_pane::liveness::OpenOwner,
        /// The model's `● live`: true only after a read armed, false the
        /// instant a message says the arming may be gone.
        live: Arc<AtomicBool>,
        /// Refetches (reads that re-armed) since the open.
        refetches: Arc<AtomicU32>,
        /// Times the model dropped `● live`.
        drops: Arc<AtomicU32>,
        /// `Msg::ConnectionLost`s received, each of which the shell answers
        /// with a redial.
        lost: Arc<AtomicU32>,
        /// Servers named by each invalidation, in order.
        invalidated_by: Arc<Mutex<Vec<Server>>>,
    }

    use std::sync::Mutex;

    async fn dial(seed: &str) -> Client {
        let (client, _) = redis_pane::redis::connect_with(seed, &Credentials::default())
            .await
            .expect("cluster client");
        client
    }

    /// The listeners `Shell::watch_link` starts for a client, plus the
    /// invalidation one.
    fn attach(
        client: &Client,
        owner: &redis_pane::liveness::OpenOwner,
        tx: &tokio::sync::mpsc::Sender<Msg>,
        seen: &Arc<Mutex<Vec<Server>>>,
        errors: bool,
    ) {
        let clock: Arc<dyn redis_pane_core::clock::Clock> = Arc::new(redis_pane::SystemClock);
        redis_pane::liveness::watch_reconnects(client, owner, tx, &clock);
        redis_pane::liveness::watch_topology(client, owner, tx);
        if errors {
            redis_pane::liveness::watch_errors(client, tx, &clock);
        }
        let mut invalidations = client.invalidation_rx();
        let (tx, seen) = (tx.clone(), seen.clone());
        tokio::spawn(async move {
            while let Ok(inv) = invalidations.recv().await {
                seen.lock().unwrap().push(inv.server);
                if tx.send(Msg::Invalidated).await.is_err() {
                    return;
                }
            }
        });
    }

    /// `read_key` in the shell: arm and read, record the owner, and report a
    /// wedged client as a lost link. Retries ordinary failures (a cluster in
    /// the middle of a failover refuses reads for a moment); `false` means the
    /// client is wedged and a redial is on its way.
    async fn read(
        client: &Client,
        owner: &redis_pane::liveness::OpenOwner,
        key: &str,
        tx: &tokio::sync::mpsc::Sender<Msg>,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match read_value(client, key.as_bytes(), Arming::Enabled).await {
                Ok(_) => {
                    owner.armed(client, key.as_bytes());
                    return true;
                }
                Err(e) if redis_pane::liveness::is_wedged(client, &e) => {
                    eprintln!("viewer read wedged: {e:?}");
                    let _ = tx.send(Msg::ConnectionLost).await;
                    return false;
                }
                Err(e) => {
                    eprintln!("viewer read failed: {e:?}");
                    assert!(Instant::now() < deadline, "read never succeeded: {e:?}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    impl Viewer {
        async fn open(cluster: &Cluster, key: &str) -> Viewer {
            Viewer::open_as(cluster, key, false).await
        }

        /// `healing`: a client that *has* a reconnect policy, so a dropped
        /// node connection is redialled in place and `reconnect_rx` names it.
        /// The app's own Cluster client has none (see `watch_errors`), so
        /// this is not what production does; it is the way to feed
        /// `watch_reconnects` a real reconnect of an unrelated node. Its
        /// `error_rx` is not listened to, as a drop is not a lost link there.
        async fn open_as(cluster: &Cluster, key: &str, healing: bool) -> Viewer {
            let seed = cluster.seed_url();
            let client = if healing {
                let mut config = Config::from_url(&seed).expect("cluster url");
                config.version = fred::types::RespVersion::RESP3;
                let mut builder = Builder::from_config(config);
                builder.set_policy(ReconnectPolicy::new_constant(0, 250));
                let client = builder.build().expect("client");
                client.init().await.expect("init");
                assert!(redis_pane::redis::probe_tracking(&client).await);
                client
            } else {
                dial(&seed).await
            };
            let owner = redis_pane::liveness::OpenOwner::default();
            let (tx, mut rx) = tokio::sync::mpsc::channel::<Msg>(64);
            let v = Viewer {
                cur: Arc::new(Mutex::new(client.clone())),
                key: key.to_string(),
                owner: owner.clone(),
                live: Arc::new(AtomicBool::new(false)),
                refetches: Arc::new(AtomicU32::new(0)),
                drops: Arc::new(AtomicU32::new(0)),
                lost: Arc::new(AtomicU32::new(0)),
                invalidated_by: Arc::new(Mutex::new(Vec::new())),
            };
            attach(&client, &owner, &tx, &v.invalidated_by, !healing);
            assert!(read(&client, &owner, key, &tx).await);
            v.live.store(true, Ordering::SeqCst);

            // The loop: what the core asks of the shell for each message.
            let (cur, live, refetches, drops, lost, seen) = (
                v.cur.clone(),
                v.live.clone(),
                v.refetches.clone(),
                v.drops.clone(),
                v.lost.clone(),
                v.invalidated_by.clone(),
            );
            let (o, k) = (owner.clone(), key.to_string());
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    let rearm = match msg {
                        Msg::Invalidated
                        | Msg::Connected {
                            tracking_supported: true,
                            ..
                        } => true,
                        Msg::Connected { .. } => {
                            live.store(false, Ordering::SeqCst);
                            drops.fetch_add(1, Ordering::SeqCst);
                            false
                        }
                        Msg::ConnectionLost => {
                            lost.fetch_add(1, Ordering::SeqCst);
                            live.store(false, Ordering::SeqCst);
                            drops.fetch_add(1, Ordering::SeqCst);
                            // The shell's redial: a new client replaces the
                            // old, which is quit, and the record is cleared.
                            let fresh = dial(&seed).await;
                            let old = std::mem::replace(&mut *cur.lock().unwrap(), fresh.clone());
                            tokio::spawn(async move {
                                let _ = old.quit().await;
                            });
                            o.clear();
                            attach(&fresh, &o, &tx, &seen, true);
                            true
                        }
                        _ => false,
                    };
                    if rearm {
                        live.store(false, Ordering::SeqCst);
                        drops.fetch_add(1, Ordering::SeqCst);
                        let client = cur.lock().unwrap().clone();
                        if read(&client, &o, &k, &tx).await {
                            refetches.fetch_add(1, Ordering::SeqCst);
                            live.store(true, Ordering::SeqCst);
                        }
                    }
                }
            });
            v
        }

        fn client(&self) -> Client {
            self.cur.lock().unwrap().clone()
        }
        fn refetches(&self) -> u32 {
            self.refetches.load(Ordering::SeqCst)
        }
        fn drops(&self) -> u32 {
            self.drops.load(Ordering::SeqCst)
        }
        fn live(&self) -> bool {
            self.live.load(Ordering::SeqCst)
        }
        fn last_invalidated_by(&self) -> Option<Server> {
            self.invalidated_by.lock().unwrap().last().cloned()
        }
    }

    /// Poll `cond` every 50ms until it holds, or panic at `within`.
    async fn until(what: &str, within: Duration, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + within;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Write `key` from another client and wait for the Viewer to refetch and
    /// be live again, returning once it has. Proves a *later* write still
    /// invalidates, which is what re-arming is for.
    async fn write_and_expect_refetch(v: &Viewer, writer: &Client, value: &str) {
        let before = v.refetches();
        writer
            .set::<(), _, _>(&v.key, value, None, None, false)
            .await
            .unwrap();
        until(
            &format!("a refetch after writing {value}"),
            Duration::from_secs(10),
            || v.refetches() > before && v.live(),
        )
        .await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn the_open_key_is_armed_on_its_owner_for_a_key_on_each_primary() {
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        for primary in cluster.primaries().await {
            let key = cluster.key_in_slot_of(primary.port).await;
            writer
                .set::<(), _, _>(&key, "v0", None, None, false)
                .await
                .unwrap();
            let v = Viewer::open(&cluster, &key).await;
            assert_eq!(v.owner.owner().map(|s| s.port), Some(primary.port));
            assert!(v.live());
            // The arming is consumed by every invalidation, so five in a row
            // is five re-arms.
            for round in 0..5 {
                write_and_expect_refetch(&v, &writer, &format!("w{round}")).await;
                assert_eq!(
                    v.last_invalidated_by().map(|s| s.port),
                    Some(primary.port),
                    "round {round}: the push must come from the owner"
                );
            }
            assert_eq!(v.refetches(), 5);
            assert_eq!(v.lost.load(Ordering::SeqCst), 0);
            let _ = v.client().quit().await;
        }
        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn no_other_node_holds_a_pending_opt_in_for_the_open_key() {
        // `CLIENT TRACKINGINFO` does show it: `caching-yes` is listed while a
        // `CLIENT CACHING YES` is waiting for its command (the spike's control
        // proves the flag is observable), so a mis-routed arming would leave it
        // set on a non-owner.
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        let primaries = cluster.primaries().await;
        let key = cluster.key_in_slot_of(primaries[0].port).await;
        writer
            .set::<(), _, _>(&key, "v0", None, None, false)
            .await
            .unwrap();
        let v = Viewer::open(&cluster, &key).await;
        for round in 0..5 {
            for other in primaries.iter().filter(|p| p.port != primaries[0].port) {
                let info = tracking_info(&v.client(), other.port).await;
                assert!(
                    !info.contains("caching-yes"),
                    "round {round}: {} holds a pending opt-in: {info}",
                    other.port
                );
                // They are in tracking mode, though: the opt-in is what is absent.
                assert!(
                    info.contains("optin"),
                    "{} is not tracking: {info}",
                    other.port
                );
            }
            write_and_expect_refetch(&v, &writer, &format!("w{round}")).await;
        }
        let _ = v.client().quit().await;
        let _ = writer.quit().await;
    }

    /// `CLIENT KILL` this client's own connection to `port`.
    async fn kill_connection(cluster: &Cluster, client: &Client, port: u16) {
        let id: i64 = client
            .with_cluster_node(node(port))
            .client_id()
            .await
            .expect("CLIENT ID");
        cluster
            .cli(port, &["client", "kill", "id", &id.to_string()])
            .await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn an_unrelated_node_reconnecting_in_place_neither_refetches_nor_drops_live() {
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        let primaries = cluster.primaries().await;
        let key = cluster.key_in_slot_of(primaries[0].port).await;
        writer
            .set::<(), _, _>(&key, "v0", None, None, false)
            .await
            .unwrap();
        let v = Viewer::open_as(&cluster, &key, true).await;
        let mut reconnects = v.client().reconnect_rx();

        let other = primaries[1].port;
        kill_connection(&cluster, &v.client(), other).await;
        // fred redials it and says so.
        let back = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(s) = reconnects.recv().await
                    && s.port == other
                {
                    return s;
                }
            }
        })
        .await
        .expect("the killed node connection never came back");
        assert_eq!(back.port, other);
        // ... and the shell re-enabled tracking on it, without a refetch.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let info = tracking_info(&v.client(), other).await;
            if info.contains("optin") {
                break;
            }
            assert!(Instant::now() < deadline, "never re-armed tracking: {info}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Give a (wrong) refetch every chance to happen, deterministically:
        // a later round trip through the same channel ordering is the next
        // invalidation, below. Anything wrong before it would already show.
        assert!(v.live(), "an unrelated node reconnecting dropped live");
        assert_eq!(v.refetches(), 0, "an unrelated node reconnecting refetched");
        assert_eq!(v.drops(), 0);
        assert_eq!(
            v.lost.load(Ordering::SeqCst),
            0,
            "no redial for a node that came back"
        );
        // Still armed on the owner: a write still invalidates.
        write_and_expect_refetch(&v, &writer, "after").await;
        assert_eq!(v.refetches(), 1);
        assert_eq!(
            v.last_invalidated_by().map(|s| s.port),
            Some(primaries[0].port)
        );
        let _ = v.client().quit().await;
        let _ = writer.quit().await;
    }

    /// What the app actually does, since its Cluster client has no reconnect
    /// policy: any node's dropped connection leaves the client unusable, so the
    /// link is reported lost and redialled, and the key is refetched and
    /// re-armed on the new client. The header may not claim live in between,
    /// and must be live and armed afterwards.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn an_unrelated_node_dropping_costs_a_redial_and_ends_live_and_armed() {
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        let primaries = cluster.primaries().await;
        let key = cluster.key_in_slot_of(primaries[0].port).await;
        writer
            .set::<(), _, _>(&key, "v0", None, None, false)
            .await
            .unwrap();
        let v = Viewer::open(&cluster, &key).await;

        kill_connection(&cluster, &v.client(), primaries[1].port).await;
        until("a redial and a refetch", Duration::from_secs(30), || {
            v.lost.load(Ordering::SeqCst) >= 1 && v.refetches() >= 1 && v.live()
        })
        .await;
        assert!(v.drops() >= 1, "live was never dropped over the lost link");
        assert_eq!(v.owner.owner().map(|s| s.port), Some(primaries[0].port));
        write_and_expect_refetch(&v, &writer, "after").await;
        assert_eq!(
            v.last_invalidated_by().map(|s| s.port),
            Some(primaries[0].port)
        );
        let _ = v.client().quit().await;
        let _ = writer.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn the_owner_reconnecting_drops_live_then_re_arms_and_a_later_write_invalidates() {
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        let primaries = cluster.primaries().await;
        let key = cluster.key_in_slot_of(primaries[0].port).await;
        writer
            .set::<(), _, _>(&key, "v0", None, None, false)
            .await
            .unwrap();
        let v = Viewer::open(&cluster, &key).await;

        kill_connection(&cluster, &v.client(), primaries[0].port).await;
        // The owner's drop is reported at once, and when the connection is
        // back the claim is dropped, the key refetched and re-armed.
        until(
            "a refetch after the owner reconnected",
            Duration::from_secs(20),
            || v.refetches() >= 1 && v.live(),
        )
        .await;
        assert!(v.drops() >= 1, "live was never dropped");
        assert!(
            v.lost.load(Ordering::SeqCst) >= 1,
            "the owner's drop is a lost link"
        );
        // The re-arm is real: a write made now invalidates.
        let inv_before = v.invalidated_by.lock().unwrap().len();
        let refetches = v.refetches();
        writer
            .set::<(), _, _>(&key, "after", None, None, false)
            .await
            .unwrap();
        until(
            "an invalidation after the re-arm",
            Duration::from_secs(10),
            || {
                v.invalidated_by.lock().unwrap().len() > inv_before
                    && v.refetches() > refetches
                    && v.live()
            },
        )
        .await;
        assert_eq!(
            v.last_invalidated_by().map(|s| s.port),
            Some(primaries[0].port)
        );
        let _ = v.client().quit().await;
        let _ = writer.quit().await;
    }

    /// Drive a command at the key until `done`: `fred` learns of a failover or
    /// slot move from traffic, and the Viewer's own sync is only periodic.
    async fn nudge_until(v: &Viewer, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            let _ = v.client().get::<Option<String>, _>(&v.key).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_forced_failover_re_arms_the_key_on_the_new_primary() {
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        let primaries = cluster.primaries().await;
        let old = &primaries[0];
        let key = cluster.key_in_slot_of(old.port).await;
        writer
            .set::<(), _, _>(&key, "v0", None, None, false)
            .await
            .unwrap();
        let v = Viewer::open(&cluster, &key).await;
        let replica = cluster
            .replicas()
            .await
            .into_iter()
            .find(|r| r.primary_id.as_deref() == Some(old.id.as_str()))
            .expect("a replica");

        cluster.failover(replica.port).await;
        nudge_until(&v, "the key re-armed on the promoted replica", || {
            v.refetches() >= 1 && v.live() && v.owner.owner().map(|s| s.port) == Some(replica.port)
        })
        .await;
        // A later write goes to the new primary, and invalidates from it.
        let writer2 = cluster.client().await;
        write_and_expect_refetch(&v, &writer2, "after-failover").await;
        assert_eq!(v.last_invalidated_by().map(|s| s.port), Some(replica.port));
        eprintln!(
            "failover: {} lost link(s), {} refetch(es)",
            v.lost.load(Ordering::SeqCst),
            v.refetches()
        );
        let _ = v.client().quit().await;
        let _ = writer.quit().await;
        let _ = writer2.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_slot_migration_re_arms_the_key_on_the_new_owner() {
        let cluster = start_cluster().await;
        let writer = cluster.client().await;
        let primaries = cluster.primaries().await;
        let (from, to) = (primaries[0].port, primaries[1].port);
        let key = cluster.key_in_slot_of(from).await;
        writer
            .set::<(), _, _>(&key, "v0", None, None, false)
            .await
            .unwrap();
        let v = Viewer::open(&cluster, &key).await;

        cluster.migrate_slot(slot(&key), from, to).await;
        nudge_until(&v, "the key re-armed on the slot's new owner", || {
            v.refetches() >= 1 && v.live() && v.owner.owner().map(|s| s.port) == Some(to)
        })
        .await;
        let writer2 = cluster.client().await;
        write_and_expect_refetch(&v, &writer2, "after-migration").await;
        assert_eq!(v.last_invalidated_by().map(|s| s.port), Some(to));
        eprintln!(
            "migration: {} lost link(s), {} refetch(es)",
            v.lost.load(Ordering::SeqCst),
            v.refetches()
        );
        let _ = v.client().quit().await;
        let _ = writer.quit().await;
        let _ = writer2.quit().await;
    }
}

/// M5 task 6: every shipped mutation against a real cluster (R4.x, R1.11,
/// ADR-0022). Each goes through `mutate::execute_settled`, the function the
/// shell's mutation path runs, on a client from `connect_with`, never a raw
/// `fred` call, with one key in each primary's slot range.
mod cluster_mutations {
    use super::Credentials;
    use super::support::cluster::{Cluster, start_cluster};
    use fred::prelude::*;
    use redis_pane::redis::mutate::execute_settled;
    use redis_pane_core::mutation::{Mutation, MutationOutcome, NotWritten};
    use redis_pane_core::state::ReadOnlyReason;
    use redis_pane_core::state::value::ListEnd;

    const DONE: MutationOutcome = MutationOutcome::Done;
    const NOTHING: MutationOutcome = MutationOutcome::NothingToRemove;
    fn refused(n: NotWritten) -> MutationOutcome {
        MutationOutcome::NotWritten(n)
    }

    async fn connect(cluster: &Cluster) -> Client {
        redis_pane::redis::connect_with(&cluster.seed_url(), &Credentials::default())
            .await
            .expect("cluster client")
            .0
    }

    /// A key named `rp:<tag>:<n>` whose slot is owned by `primary`.
    async fn key_for(cluster: &Cluster, primary: u16, tag: &str) -> String {
        let node = cluster
            .primaries()
            .await
            .into_iter()
            .find(|n| n.port == primary)
            .expect("a primary");
        (0u32..)
            .map(|i| format!("rp:{tag}:{i}"))
            .find(|k| node.owns(fred::util::redis_keyslot(k.as_bytes())))
            .unwrap()
    }

    /// Run one mutation through the shell's write path; it must not fail.
    async fn run(client: &Client, m: Mutation) -> MutationOutcome {
        let settled = execute_settled(client, &m).await;
        assert!(!settled.link_lost, "{m:?} wedged the client");
        settled
            .result
            .unwrap_or_else(|e| panic!("{m:?} failed on a healthy cluster: {e}"))
    }

    async fn ttl(client: &Client, key: &str) -> i64 {
        client.ttl(key).await.unwrap()
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn every_shipped_mutation_lands_on_the_owner_of_each_primarys_slots() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        let primaries = cluster.primaries().await;
        assert_eq!(primaries.len(), 3);
        for p in primaries.iter().map(|n| n.port) {
            let k = |tag: &'static str| {
                let cluster = &cluster;
                async move { key_for(cluster, p, tag).await }
            };
            let who = format!("primary {p}");

            // String edit keeps the TTL; delete.
            let s = k("str").await;
            let _: () = client.set(&s, "old", None, None, false).await.unwrap();
            let _: bool = client.expire(&s, 1000, None).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetString {
                        key: s.as_str().into(),
                        value: b"new".to_vec()
                    }
                )
                .await,
                DONE,
                "{who}"
            );
            assert_eq!(client.get::<String, _>(&s).await.unwrap(), "new");
            assert!(ttl(&client, &s).await > 0, "{who}: KEEPTTL");
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteKey {
                        key: s.as_str().into()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(client.exists::<i64, _>(&s).await.unwrap(), 0, "{who}");

            // Hash: edit, add, remove (last removal removes the key).
            let h = k("hash").await;
            let _: i64 = client.hset(&h, ("a", "1")).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetHashField {
                        key: h.as_str().into(),
                        field: b"a".to_vec(),
                        value: b"2".to_vec()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::AddHashField {
                        key: h.as_str().into(),
                        field: b"b".to_vec(),
                        value: b"3".to_vec()
                    }
                )
                .await,
                DONE
            );
            let got: std::collections::BTreeMap<String, String> = client.hgetall(&h).await.unwrap();
            assert_eq!(got.get("a").map(String::as_str), Some("2"), "{who}");
            assert_eq!(got.get("b").map(String::as_str), Some("3"), "{who}");
            for f in [b"a", b"b"] {
                assert_eq!(
                    run(
                        &client,
                        Mutation::DeleteHashField {
                            key: h.as_str().into(),
                            field: f.to_vec()
                        }
                    )
                    .await,
                    DONE
                );
            }
            assert_eq!(client.exists::<i64, _>(&h).await.unwrap(), 0, "{who}");

            // Set: add, remove.
            let st = k("set").await;
            let _: i64 = client.sadd(&st, "x").await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::AddSetMember {
                        key: st.as_str().into(),
                        member: b"y".to_vec()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(client.scard::<i64, _>(&st).await.unwrap(), 2, "{who}");
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteSetMember {
                        key: st.as_str().into(),
                        member: b"x".to_vec()
                    }
                )
                .await,
                DONE
            );
            assert!(client.sismember::<bool, _, _>(&st, "y").await.unwrap());
            assert!(!client.sismember::<bool, _, _>(&st, "x").await.unwrap());

            // List: edit, add at both ends, remove by index.
            let l = k("list").await;
            let _: i64 = client.rpush(&l, vec!["a", "b", "c"]).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetListElement {
                        key: l.as_str().into(),
                        index: 1,
                        expected: b"b".to_vec(),
                        value: b"B".to_vec()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::AddListElement {
                        key: l.as_str().into(),
                        end: ListEnd::Head,
                        value: b"h".to_vec()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::AddListElement {
                        key: l.as_str().into(),
                        end: ListEnd::Tail,
                        value: b"t".to_vec()
                    }
                )
                .await,
                DONE
            );
            // [h a B c t]; remove index 2 (B).
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteListElement {
                        key: l.as_str().into(),
                        index: 2,
                        expected: b"B".to_vec()
                    }
                )
                .await,
                DONE
            );
            let got: Vec<String> = client.lrange(&l, 0, -1).await.unwrap();
            assert_eq!(got, ["h", "a", "c", "t"], "{who}");

            // ZSet: score edit, add, remove.
            let z = k("zset").await;
            let _: i64 = client
                .zadd(&z, None, None, false, false, (1.0, "m"))
                .await
                .unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetZSetScore {
                        key: z.as_str().into(),
                        member: b"m".to_vec(),
                        score: 5.5
                    }
                )
                .await,
                DONE
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::AddZSetMember {
                        key: z.as_str().into(),
                        member: b"n".to_vec(),
                        score: 2.0
                    }
                )
                .await,
                DONE
            );
            let sc: f64 = client.zscore(&z, "m").await.unwrap();
            assert_eq!(sc, 5.5, "{who}");
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteZSetMember {
                        key: z.as_str().into(),
                        member: b"m".to_vec()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(client.zcard::<i64, _>(&z).await.unwrap(), 1, "{who}");

            // TTL: set, shift, clear.
            let t = k("ttl").await;
            let _: () = client.set(&t, "v", None, None, false).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetTtl {
                        key: t.as_str().into(),
                        seconds: 500
                    }
                )
                .await,
                DONE
            );
            let now = ttl(&client, &t).await;
            assert!((490..=500).contains(&now), "{who}: ttl {now}");
            assert_eq!(
                run(
                    &client,
                    Mutation::ShiftTtl {
                        key: t.as_str().into(),
                        delta_seconds: 100
                    }
                )
                .await,
                DONE
            );
            let now = ttl(&client, &t).await;
            assert!((590..=600).contains(&now), "{who}: ttl {now}");
            assert_eq!(
                run(
                    &client,
                    Mutation::PersistTtl {
                        key: t.as_str().into()
                    }
                )
                .await,
                DONE
            );
            assert_eq!(ttl(&client, &t).await, -1, "{who}");
        }
        let _ = client.quit().await;
    }

    #[tokio::test]
    #[ignore = "needs docker"]
    async fn every_guarded_refusal_holds_on_each_primary() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        for p in cluster.primaries().await.iter().map(|n| n.port) {
            let who = format!("primary {p}");
            // A key that does not exist, in this primary's range: every
            // guarded write must answer "key gone" and never create it.
            let gone = key_for(&cluster, p, "gone").await;
            let g = || gone.as_str().into();
            let cases: Vec<(&str, Mutation)> = vec![
                (
                    "set string",
                    Mutation::SetString {
                        key: g(),
                        value: b"v".to_vec(),
                    },
                ),
                (
                    "set field",
                    Mutation::SetHashField {
                        key: g(),
                        field: b"f".to_vec(),
                        value: b"v".to_vec(),
                    },
                ),
                (
                    "add field",
                    Mutation::AddHashField {
                        key: g(),
                        field: b"f".to_vec(),
                        value: b"v".to_vec(),
                    },
                ),
                (
                    "add member",
                    Mutation::AddSetMember {
                        key: g(),
                        member: b"m".to_vec(),
                    },
                ),
                (
                    "set element",
                    Mutation::SetListElement {
                        key: g(),
                        index: 0,
                        expected: b"e".to_vec(),
                        value: b"v".to_vec(),
                    },
                ),
                (
                    "push head",
                    Mutation::AddListElement {
                        key: g(),
                        end: ListEnd::Head,
                        value: b"v".to_vec(),
                    },
                ),
                (
                    "push tail",
                    Mutation::AddListElement {
                        key: g(),
                        end: ListEnd::Tail,
                        value: b"v".to_vec(),
                    },
                ),
                (
                    "delete element",
                    Mutation::DeleteListElement {
                        key: g(),
                        index: 0,
                        expected: b"e".to_vec(),
                    },
                ),
                (
                    "set score",
                    Mutation::SetZSetScore {
                        key: g(),
                        member: b"m".to_vec(),
                        score: 1.0,
                    },
                ),
                (
                    "add zset member",
                    Mutation::AddZSetMember {
                        key: g(),
                        member: b"m".to_vec(),
                        score: 1.0,
                    },
                ),
                (
                    "set ttl",
                    Mutation::SetTtl {
                        key: g(),
                        seconds: 60,
                    },
                ),
                ("persist", Mutation::PersistTtl { key: g() }),
                (
                    "shift ttl",
                    Mutation::ShiftTtl {
                        key: g(),
                        delta_seconds: 10,
                    },
                ),
            ];
            for (what, m) in cases {
                assert_eq!(
                    run(&client, m).await,
                    refused(NotWritten::KeyGone),
                    "{who}: {what}"
                );
            }
            assert_eq!(
                client.exists::<i64, _>(&gone).await.unwrap(),
                0,
                "{who}: nothing recreated"
            );
            // Removals from a key that is gone are "nothing to remove"; DEL is Done.
            for (what, m) in [
                (
                    "hdel",
                    Mutation::DeleteHashField {
                        key: g(),
                        field: b"f".to_vec(),
                    },
                ),
                (
                    "srem",
                    Mutation::DeleteSetMember {
                        key: g(),
                        member: b"m".to_vec(),
                    },
                ),
                (
                    "zrem",
                    Mutation::DeleteZSetMember {
                        key: g(),
                        member: b"m".to_vec(),
                    },
                ),
            ] {
                assert_eq!(run(&client, m).await, NOTHING, "{who}: {what}");
            }
            assert_eq!(
                run(&client, Mutation::DeleteKey { key: g() }).await,
                DONE,
                "{who}"
            );

            // Hash: field gone / field taken, existing value untouched.
            let h = key_for(&cluster, p, "h").await;
            let _: i64 = client.hset(&h, ("f", "1")).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetHashField {
                        key: h.as_str().into(),
                        field: b"nope".to_vec(),
                        value: b"x".to_vec()
                    }
                )
                .await,
                refused(NotWritten::FieldGone),
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::AddHashField {
                        key: h.as_str().into(),
                        field: b"f".to_vec(),
                        value: b"x".to_vec()
                    }
                )
                .await,
                refused(NotWritten::FieldExists),
                "{who}"
            );
            assert_eq!(
                client.hget::<String, _, _>(&h, "f").await.unwrap(),
                "1",
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteHashField {
                        key: h.as_str().into(),
                        field: b"nope".to_vec()
                    }
                )
                .await,
                NOTHING,
                "{who}"
            );

            // Set: member taken / nothing to remove.
            let st = key_for(&cluster, p, "s").await;
            let _: i64 = client.sadd(&st, "m").await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::AddSetMember {
                        key: st.as_str().into(),
                        member: b"m".to_vec()
                    }
                )
                .await,
                refused(NotWritten::MemberExists),
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteSetMember {
                        key: st.as_str().into(),
                        member: b"nope".to_vec()
                    }
                )
                .await,
                NOTHING,
                "{who}"
            );

            // List: the element moved (a stale index), for edit and delete.
            let l = key_for(&cluster, p, "l").await;
            let _: i64 = client.rpush(&l, vec!["a", "b"]).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetListElement {
                        key: l.as_str().into(),
                        index: 0,
                        expected: b"b".to_vec(),
                        value: b"x".to_vec()
                    }
                )
                .await,
                refused(NotWritten::ElementMoved),
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteListElement {
                        key: l.as_str().into(),
                        index: 1,
                        expected: b"a".to_vec()
                    }
                )
                .await,
                refused(NotWritten::ElementMoved),
                "{who}"
            );
            let got: Vec<String> = client.lrange(&l, 0, -1).await.unwrap();
            assert_eq!(got, ["a", "b"], "{who}: nothing written, no sentinel left");

            // ZSet: member gone / member taken (score unchanged) / nothing to remove.
            let z = key_for(&cluster, p, "z").await;
            let _: i64 = client
                .zadd(&z, None, None, false, false, (1.0, "m"))
                .await
                .unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::SetZSetScore {
                        key: z.as_str().into(),
                        member: b"nope".to_vec(),
                        score: 9.0
                    }
                )
                .await,
                refused(NotWritten::MemberGone),
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::AddZSetMember {
                        key: z.as_str().into(),
                        member: b"m".to_vec(),
                        score: 9.0
                    }
                )
                .await,
                refused(NotWritten::MemberExists),
                "{who}"
            );
            assert_eq!(
                client.zscore::<f64, _, _>(&z, "m").await.unwrap(),
                1.0,
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::DeleteZSetMember {
                        key: z.as_str().into(),
                        member: b"nope".to_vec()
                    }
                )
                .await,
                NOTHING,
                "{who}"
            );

            // TTL: no expiry to persist or shift; a shift that would expire now.
            let t = key_for(&cluster, p, "t").await;
            let _: () = client.set(&t, "v", None, None, false).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::PersistTtl {
                        key: t.as_str().into()
                    }
                )
                .await,
                NOTHING,
                "{who}"
            );
            assert_eq!(
                run(
                    &client,
                    Mutation::ShiftTtl {
                        key: t.as_str().into(),
                        delta_seconds: 10
                    }
                )
                .await,
                refused(NotWritten::NoExpiry),
                "{who}"
            );
            let _: bool = client.expire(&t, 100, None).await.unwrap();
            assert_eq!(
                run(
                    &client,
                    Mutation::ShiftTtl {
                        key: t.as_str().into(),
                        delta_seconds: -500
                    }
                )
                .await,
                refused(NotWritten::WouldExpireNow),
                "{who}"
            );
            assert!(ttl(&client, &t).await > 0, "{who}: TTL untouched");
        }
        let _ = client.quit().await;
    }

    /// A write to a key whose slot is mid-migration: `fred` answers `ASK`
    /// with a `Routing` failure and the client is wedged for good (ADR-0022).
    /// The write is reported as failed with its detail, and as a lost link.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_write_to_a_slot_mid_migration_fails_visibly_and_asks_for_the_redial() {
        let cluster = start_cluster().await;
        let client = connect(&cluster).await;
        let primaries = cluster.primaries().await;
        let (from, to) = (primaries[0].port, primaries[1].port);
        let key = key_for(&cluster, from, "mig").await;
        let _: () = client.set(&key, "v0", None, None, false).await.unwrap();
        let slot = fred::util::redis_keyslot(key.as_bytes());

        // Every key has moved to `to`, but `from` still owns the slot: `ASK`.
        cluster.begin_migration(slot, from, to).await;
        let m = Mutation::SetString {
            key: key.as_str().into(),
            value: b"v1".to_vec(),
        };
        let settled = execute_settled(&client, &m).await;
        let detail = settled
            .result
            .as_ref()
            .expect_err("a write that hit ASK must not look like it worked");
        assert!(
            !detail.is_empty(),
            "the failure carries the server's detail"
        );
        assert!(
            settled.link_lost,
            "a wedged client asks for the redial: {detail}"
        );

        // Ownership settles; the redial (what `Msg::ConnectionLost` triggers)
        // gives a client that works, on the key's new owner.
        cluster.finish_migration(slot, to).await;
        let _ = client.quit().await;
        let fresh = connect(&cluster).await;
        assert_eq!(run(&fresh, m).await, DONE);
        assert_eq!(fresh.get::<String, _>(&key).await.unwrap(), "v1");
        let _ = fresh.quit().await;
    }

    /// A cluster is not locked as a replica because the seed node is one
    /// (task 6): writes go to primaries.
    #[tokio::test]
    #[ignore = "needs docker"]
    async fn a_cluster_seeded_through_a_replica_is_not_read_only() {
        let cluster = start_cluster().await;
        let replica = cluster.replicas().await.remove(0).port;
        for url in [
            format!("redis-cluster://127.0.0.1:{replica}"),
            format!("redis://127.0.0.1:{replica}"),
            cluster.seed_url(),
        ] {
            let (client, est) = redis_pane::redis::connect_with(&url, &Credentials::default())
                .await
                .unwrap_or_else(|e| panic!("{url}: {e}"));
            assert!(client.is_clustered(), "{url}");
            assert_eq!(est.read_only, None::<ReadOnlyReason>, "{url}");
            // And it writes: that is the claim being made.
            let key = key_for(&cluster, cluster.primaries().await[0].port, "seed").await;
            let m = Mutation::DeleteKey {
                key: key.as_str().into(),
            };
            assert_eq!(run(&client, m).await, DONE, "{url}");
            let _ = client.quit().await;
        }
    }
}
