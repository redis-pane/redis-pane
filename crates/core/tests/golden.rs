//! Golden-frame tests (ADR-0011).
//!
//! A frame is compared against a committed fixture. When a change is
//! intentional, re-record with:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p redis-pane-core
//! ```
//!
//! and review the diff — a rendered terminal is the most reviewable artifact
//! this project produces, so accepting a change should be cheap but never
//! automatic.

use std::path::PathBuf;

use ratatui::layout::Rect;
use redis_pane_core::clock::{Clock, FixedClock};
use redis_pane_core::render;
use redis_pane_core::state::{Connection, Environment, Link, Source, State, Tracking};
use redis_pane_core::theme::{ColorDepth, Theme};

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.txt"))
}

/// Compare `actual` against the committed fixture for `name`.
fn assert_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!("missing fixture {}\n\nrecord it with:\n  UPDATE_GOLDEN=1 cargo test -p redis-pane-core\n\nwould have written:\n{actual}", path.display())
    });
    if expected != actual {
        let side_by_side = expected
            .lines()
            .zip(actual.lines())
            .filter(|(e, a)| e != a)
            .map(|(e, a)| format!("  expected | {e}\n  actual   | {a}"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "golden frame {name} changed:\n{side_by_side}\n\nif intended: UPDATE_GOLDEN=1 cargo test -p redis-pane-core"
        );
    }
}

fn staging_state() -> State {
    State {
        cols: 100,
        rows: 2,
        connection: Connection {
            target: "cache-01:6379/0".into(),
            environment: Environment::Staging,
            source: Source::Profile("staging".into()),
            topology: None,
        },
        last_read_ms: Some(60_000),
        link: Link::Up {
            version: "8.4.0".into(),
            tracking: Tracking::Armed,
        },
        ..State::default()
    }
}

fn render_at(depth: ColorDepth, clock: &dyn Clock) -> String {
    let buf = render::frame(
        &staging_state(),
        &Theme::new(depth),
        clock,
        Rect::new(0, 0, 100, 2),
    );
    render::to_golden(&buf)
}

// ── M0.3 — theme tokens degrade across colour depths ────────────────────────

#[test]
fn golden_title_bar_truecolor() {
    assert_golden(
        "title_bar_truecolor",
        &render_at(ColorDepth::TrueColor, &FixedClock(74_000)),
    );
}

#[test]
fn golden_title_bar_ansi256() {
    assert_golden(
        "title_bar_ansi256",
        &render_at(ColorDepth::Ansi256, &FixedClock(74_000)),
    );
}

#[test]
fn golden_title_bar_monochrome() {
    assert_golden(
        "title_bar_monochrome",
        &render_at(ColorDepth::Monochrome, &FixedClock(74_000)),
    );
}

#[test]
fn colour_depth_changes_the_frame_but_never_the_text() {
    let t = render_at(ColorDepth::TrueColor, &FixedClock(74_000));
    let m = render_at(ColorDepth::Monochrome, &FixedClock(74_000));
    assert_ne!(t, m, "styles must differ across depths");

    let text_of = |g: &str| g.split("--- styles ---").next().unwrap().to_string();
    assert_eq!(
        text_of(&t),
        text_of(&m),
        "losing colour must lose emphasis, never information — the words \
         `staging` and the target must survive into monochrome"
    );
}

// ── M0.2 — the clock is injected, so a frame is a function of state alone ───

/// Render in a state where the clock is actually consulted.
///
/// A `live` header states no read age — the server will say when the value
/// changes, so how long ago it was read is not the reader's problem. The clock
/// only reaches the frame when liveness is degraded, so that is the state these
/// tests must use.
fn render_manual_at(clock: &dyn Clock) -> String {
    let state = State {
        link: Link::Up {
            version: "8.4.0".into(),
            tracking: Tracking::Unsupported,
        },
        ..staging_state()
    };
    let buf = render::frame(
        &state,
        &Theme::new(ColorDepth::TrueColor),
        clock,
        Rect::new(0, 0, 100, 2),
    );
    render::to_golden(&buf)
}

#[test]
fn same_clock_reading_gives_an_identical_frame_whenever_it_is_rendered() {
    let first = render_manual_at(&FixedClock(74_000));
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let second = render_manual_at(&FixedClock(74_000));
    assert_eq!(
        first, second,
        "wall-clock time passed between these two renders; the frame must not notice"
    );
}

#[test]
fn a_different_clock_reading_does_change_the_frame() {
    // Without this, the test above would pass trivially for a frame that simply
    // ignores the clock.
    let at_14s = render_manual_at(&FixedClock(74_000));
    let at_5m = render_manual_at(&FixedClock(360_000));
    assert_ne!(
        at_14s, at_5m,
        "the read age is clock-derived and must move when the clock does"
    );
}

// ── M0.11 — every readout in DESIGN §6.8 ────────────────────────────────────

use redis_pane_core::help;
use redis_pane_core::keymap::{Action, Keymap};
use redis_pane_core::msg::{KeyCode, KeyPress};
use redis_pane_core::render::{hint_bar, status_readout};
use redis_pane_core::state::{HelpView, ReadOnlyReason, ServerCondition, Tracking as Tk};

const CLOCK: FixedClock = FixedClock(74_000);

fn base() -> State {
    State {
        cols: 100,
        rows: 2,
        connection: Connection {
            target: "cache-01:6379/0".into(),
            environment: Environment::Staging,
            source: Source::Profile("staging".into()),
            topology: None,
        },
        last_read_ms: Some(60_000),
        ..State::default()
    }
}

fn up(tracking: Tk) -> Link {
    Link::Up {
        version: "8.4.0".into(),
        tracking,
    }
}

/// The readout as a plain string, which is what a reader actually sees.
fn readout(state: &State) -> String {
    status_readout(state, &CLOCK)
        .into_iter()
        .map(|(t, _)| t)
        .collect()
}

#[test]
fn golden_title_bar_readouts() {
    let cases: Vec<(&str, State)> = vec![
        (
            "healthy and live",
            State {
                link: up(Tk::Armed),
                ..base()
            },
        ),
        (
            "tracking refused, degraded to manual",
            State {
                link: up(Tk::Unsupported),
                ..base()
            },
        ),
        (
            "connected but not yet armed",
            State {
                link: up(Tk::Available),
                ..base()
            },
        ),
        (
            "connection lost",
            State {
                link: Link::Reconnecting {
                    attempt: 2,
                    retry_in_ms: None,
                },
                ..base()
            },
        ),
        (
            "connection lost, a retry scheduled",
            State {
                link: Link::Reconnecting {
                    attempt: 2,
                    retry_in_ms: Some(4_000),
                },
                ..base()
            },
        ),
        (
            "read-only by Environment",
            State {
                link: up(Tk::Armed),
                read_only: Some(ReadOnlyReason::Environment),
                ..base()
            },
        ),
        (
            "read-only because the target is a replica",
            State {
                link: up(Tk::Armed),
                read_only: Some(ReadOnlyReason::Replica),
                ..base()
            },
        ),
        (
            "read-only by choice",
            State {
                link: up(Tk::Armed),
                read_only: Some(ReadOnlyReason::User),
                ..base()
            },
        ),
        (
            "maxmemory reached",
            State {
                link: up(Tk::Armed),
                condition: Some(ServerCondition::Oom),
                read_only: Some(ReadOnlyReason::Environment),
                ..base()
            },
        ),
        (
            "server restarting",
            State {
                link: up(Tk::Available),
                condition: Some(ServerCondition::Loading { percent: 43 }),
                ..base()
            },
        ),
    ];

    let rendered = cases
        .iter()
        .map(|(why, state)| format!("{why:<44}{}", readout(state)))
        .collect::<Vec<_>>()
        .join("\n");
    assert_golden("title_bar_readouts", &rendered);
}

#[test]
fn a_replica_is_locked_rather_than_offered_a_key_that_cannot_work() {
    let replica = State {
        link: up(Tk::Armed),
        read_only: Some(ReadOnlyReason::Replica),
        ..base()
    };
    let text = readout(&replica);
    assert!(text.contains("READ-ONLY replica"), "{text}");
    assert!(text.contains("locked"), "{text}");
    assert!(
        !text.contains("⌃R"),
        "a replica must not be offered a toggle: {text}"
    );

    let env = State {
        read_only: Some(ReadOnlyReason::Environment),
        ..replica.clone()
    };
    assert!(
        readout(&env).contains("⌃R"),
        "an Environment guard is liftable"
    );
}

#[test]
fn live_states_no_age_manual_states_one_and_reconnecting_states_the_countdown() {
    // A live header owes no age: the server will say when the value changes.
    let live = readout(&State {
        link: up(Tk::Armed),
        ..base()
    });
    assert!(!live.contains("read "), "{live}");

    // Degraded to manual, the age is the whole point.
    let manual = readout(&State {
        link: up(Tk::Unsupported),
        ..base()
    });
    assert!(manual.contains("read 14s ago"), "{manual}");

    // A dropped link with nothing scheduled — the moment right after
    // `Msg::ConnectionLost`, before the shell's first reconnect attempt has
    // failed and reported a backoff. It shows the Read age, which is what
    // ADR-0009 asks for and what is true, plus the `r` key: disconnected, `r`
    // retries the connection immediately (ADR-0009) rather than reading a now
    // -dead client, so it belongs here exactly because it does something.
    let dropped = readout(&State {
        link: Link::Reconnecting {
            attempt: 1,
            retry_in_ms: None,
        },
        ..base()
    });
    assert!(dropped.contains("read 14s ago"), "{dropped}");
    assert!(
        !dropped.contains("retry"),
        "no countdown for a retry nobody scheduled: {dropped}"
    );
    assert!(
        dropped.trim_end().ends_with("r"),
        "the age, then the reconnect key: {dropped}"
    );

    // Once a retry really is scheduled the countdown is the useful fact
    // (ADR-0009): a backoff nobody can see is a freeze wearing a different
    // name. Only `Msg::ReconnectScheduled` can reach this.
    let retrying = readout(&State {
        link: Link::Reconnecting {
            attempt: 2,
            retry_in_ms: Some(4_000),
        },
        ..base()
    });
    assert!(retrying.contains("retry 4s"), "{retrying}");
    assert!(
        !retrying.contains("read "),
        "the countdown replaces the age: {retrying}"
    );
    assert!(
        retrying.trim_end().ends_with("r"),
        "immediate retry is still on offer while a backoff counts down: {retrying}"
    );
}

#[test]
fn golden_help_overlay_frame() {
    let state = State {
        link: up(Tk::Armed),
        help: Some(HelpView {
            pane: redis_pane_core::render::layout::Pane::Keys,
        }),
        rows: 14,
        ..base()
    };
    let buf = render::frame(
        &state,
        &Theme::new(ColorDepth::Monochrome),
        &CLOCK,
        Rect::new(0, 0, 100, 14),
    );
    assert_golden("help_overlay_frame", &render::to_text(&buf));
}

// ── M0.12 — hints follow the binding, not a hard-coded label ────────────────

#[test]
fn golden_hint_bar_default_bindings() {
    let state = base();
    assert_golden("hint_bar_default", &hint_bar(&state, state.cols));
}

/// R7.5's proof: rebinding an action changes what the screen says.
#[test]
fn rebinding_quit_changes_the_hint_bar_and_the_help_overlay() {
    // Wide enough that the bar's width-fitting (M3) never has to drop
    // anything — this test is about the binding, not the fitting.
    let before = State {
        cols: 400,
        ..base()
    };
    assert!(
        hint_bar(&before, before.cols).contains("q quit"),
        "{}",
        hint_bar(&before, before.cols)
    );

    let mut keymap = Keymap::default();
    keymap.bind(Action::Quit, KeyPress::ctrl(KeyCode::Char('x')));
    let after = State {
        cols: 400,
        keymap,
        ..base()
    };

    assert!(
        hint_bar(&after, after.cols).contains("⌃X quit"),
        "{}",
        hint_bar(&after, after.cols)
    );
    assert!(
        !hint_bar(&after, after.cols).contains("q quit"),
        "the stale label must be gone"
    );
    // `help_lines` (the flat list of every binding) is gone with the
    // contextual overlay — `help::everywhere`'s "quit" row is the same proof
    // now: the rebinding shows up wherever help reads the keymap from.
    assert!(
        help::everywhere(&after)
            .iter()
            .any(|r| r.label == "quit" && r.keys == "⌃X")
    );
}

/// The same rule, applied to the readout: a rebound Refetch must show the new
/// key where the degraded header offers one.
#[test]
fn rebinding_refetch_changes_the_degraded_readout() {
    let mut keymap = Keymap::default();
    keymap.bind(Action::Refetch, KeyPress::plain(KeyCode::Char('u')));
    let state = State {
        link: up(Tk::Unsupported),
        keymap,
        ..base()
    };
    let text = readout(&state);
    assert!(text.ends_with("  u"), "{text}");
}

// ── M1.3 — the keyspace browser at every breakpoint ─────────────────────────

use redis_pane_core::state::LoadedSet;
use redis_pane_core::state::loaded::{KeyKind, TTL_NONE};

/// A browser with realistic keys, all metadata fetched.
fn browsing() -> State {
    let mut keys = LoadedSet::default();
    let rows: &[(&str, KeyKind, i32, u32)] = &[
        ("user:8812:session", KeyKind::Hash, 2_537, 2_150),
        ("user:8812:profile", KeyKind::Json, TTL_NONE, 880),
        ("user:8812:cart", KeyKind::ZSet, 720, 412),
        ("user:8813:session", KeyKind::Hash, 3_400, 1_980),
        ("cart:91af3c9d2e", KeyKind::ZSet, 720, 1_153_434),
        ("feed:global:hot", KeyKind::List, TTL_NONE, 64_512),
        ("lock:checkout:8812", KeyKind::String, 12, 41),
        ("stream:orders", KeyKind::Stream, TTL_NONE, 8_400_000),
    ];
    for (i, (name, kind, ttl, size)) in rows.iter().enumerate() {
        keys.push(name.as_bytes());
        keys.set_kind(i, *kind);
        keys.set_ttl(i, *ttl, 74); // matches CLOCK below
        keys.set_size(i, *size);
    }
    let mut state = State {
        keys,
        scan: redis_pane_core::state::ScanState::Running {
            scanned: 41_203,
            estimated_total: 180_000,
        },
        link: up(Tk::Armed),
        ..base()
    };
    // The list renders through an index vector, which has to be built from the
    // store before anything is on screen.
    state.rebuild_list();
    state
}

fn draw(state: &State, w: u16, h: u16) -> String {
    let buf = render::frame(
        state,
        &Theme::new(ColorDepth::Monochrome),
        &CLOCK,
        Rect::new(0, 0, w, h),
    );
    render::to_text(&buf)
}

/// The full four-column layout begins at 120 (DESIGN §2), not at the 118 the
/// planning mockups happened to be drawn at.
#[test]
fn golden_browser_130_cols_full_density() {
    assert_golden("browser_130", &draw(&browsing(), 130, 26));
}

#[test]
fn golden_browser_split_widened() {
    // DESIGN §2: "the split is resizable" — a reader who has nudged the
    // divider toward the Viewer sees a wider keys pane in every subsequent
    // frame, not just a one-off recalculation.
    let mut state = browsing();
    state.split_adjust = 20;
    let frame = draw(&state, 130, 26);
    assert_ne!(
        frame,
        draw(&browsing(), 130, 26),
        "the frame actually changed"
    );
    assert!(
        frame.contains("user:8812:session"),
        "still the same keyspace, just redrawn wider"
    );
    assert_golden("browser_130_widened", &frame);
}

#[test]
fn golden_browser_119_cols_sheds_size() {
    assert_golden("browser_119", &draw(&browsing(), 119, 26));
}

#[test]
fn a_keys_pane_ttl_counts_down_as_the_clock_advances_with_no_new_data() {
    // "lock:checkout:8812" was read at t=74s with a 12s TTL. Advancing the
    // clock alone — no refetch, no new Msg — must shrink the number on
    // screen, the same liveness the value pane already has (R3.9).
    let at_read_time = draw(&browsing(), 130, 26);
    let five_seconds_later = draw(&browsing(), 130, 26);
    assert_eq!(
        at_read_time, five_seconds_later,
        "same clock, same frame — this pins the baseline before the next assertion"
    );

    let buf = render::frame(
        &browsing(),
        &Theme::new(ColorDepth::Monochrome),
        &FixedClock(79_000),
        Rect::new(0, 0, 130, 26),
    );
    let ticked = render::to_text(&buf);
    assert_ne!(
        at_read_time, ticked,
        "a row with a known, positive TTL must show a smaller number once the \
         clock has moved on — the keys pane never costs a round trip for this (R3.9)"
    );
}

#[test]
fn golden_browser_100_cols() {
    assert_golden("browser_100", &draw(&browsing(), 100, 26));
}

#[test]
fn golden_browser_80_cols() {
    assert_golden("browser_80", &draw(&browsing(), 80, 26));
}

#[test]
fn golden_browser_70_cols() {
    assert_golden("browser_70", &draw(&browsing(), 70, 26));
}

#[test]
fn golden_browser_single_pane_60_cols() {
    assert_golden("browser_60", &draw(&browsing(), 60, 26));
}

#[test]
fn rendering_cost_does_not_grow_with_the_keyspace() {
    // R2.6: a million-key Loaded set must draw like a ten-key one. If this ever
    // becomes false the list has stopped being virtualized.
    let mut keys = LoadedSet::default();
    for i in 0..200_000 {
        keys.push(format!("user:{i:08}:session").as_bytes());
    }
    let mut big = State {
        keys,
        link: up(Tk::Armed),
        ..base()
    };
    big.rebuild_list();

    let started = std::time::Instant::now();
    for _ in 0..50 {
        let _ = draw(&big, 130, 26);
    }
    let per_frame = started.elapsed() / 50;
    assert!(
        per_frame < std::time::Duration::from_millis(16),
        "a frame took {per_frame:?}; a keystroke must be answerable in one frame"
    );
}

// ── M1.4 — placeholders hold their column ───────────────────────────────────

/// The same keys with nothing fetched yet.
fn pending() -> State {
    let mut keys = LoadedSet::default();
    for name in [
        "user:8812:session",
        "user:8812:profile",
        "user:8812:cart",
        "user:8813:session",
        "cart:91af3c9d2e",
        "feed:global:hot",
        "lock:checkout:8812",
        "stream:orders",
    ] {
        keys.push(name.as_bytes());
    }
    let mut state = State { keys, ..browsing() };
    state.rebuild_list();
    state
}

#[test]
fn golden_browser_metadata_pending() {
    assert_golden("browser_pending", &draw(&pending(), 130, 26));
}

/// M1.4's proof: a value arriving must land exactly where its placeholder was.
#[test]
fn metadata_arriving_does_not_shift_a_single_column() {
    let pending_frame = draw(&pending(), 130, 26);
    let filled_frame = draw(&browsing(), 130, 26);

    // `str::find` returns a *byte* offset, and the dot marker preceding a key
    // name can be "●" (3 bytes, a known type) or the pending placeholder "·"
    // (2 bytes) — both occupy exactly one terminal column, so a byte offset
    // would report a false shift between them. Column position is measured in
    // characters instead, which is what actually appears on screen.
    let column_of = |frame: &str, needle: &str| -> Vec<usize> {
        frame
            .lines()
            .filter_map(|l| l.find(needle).map(|byte_idx| l[..byte_idx].chars().count()))
            .collect::<Vec<_>>()
    };

    // The key names are the anchor: they must occupy identical columns whether
    // or not the metadata beside them has arrived.
    assert_eq!(
        column_of(&pending_frame, "user:8812:session"),
        column_of(&filled_frame, "user:8812:session"),
        "a key name moved when metadata arrived"
    );

    // And the header row is geometry, not data — it cannot move at all.
    let header_of = |frame: &str| {
        frame
            .lines()
            .find(|l| l.contains("KEY") && l.contains("TYPE"))
            .unwrap()
            .to_string()
    };
    assert_eq!(
        header_of(&pending_frame),
        header_of(&filled_frame),
        "the column header shifted, so every cell under it did too"
    );
}

// ── a key that vanished while the list was on screen (DESIGN §9) ────────────

/// `browsing()` with one key deleted underneath the reader.
fn with_a_gone_key() -> State {
    let mut state = browsing();
    // `cart:91af3c9d2e`, in the middle of the list — the position is the point:
    // the row must stay where it is rather than renumbering its neighbours.
    state.keys.set_gone(4);
    state.rebuild_list();
    state
}

#[test]
fn golden_browser_with_a_deleted_key() {
    assert_golden("browser_gone", &draw(&with_a_gone_key(), 130, 26));
}

#[test]
fn a_deleted_key_keeps_its_row_and_says_so() {
    let frame = draw(&with_a_gone_key(), 130, 26);
    let row = frame
        .lines()
        .find(|l| l.contains("cart:91af3c9d2e"))
        .expect("the row must survive its key's deletion");

    assert!(row.contains('✕'), "the deletion must be marked: {row}");
    assert!(
        row.contains("gone"),
        "the TYPE column must say what happened rather than fall back to the \
         pending placeholder, which would claim the fetch had not happened yet"
    );
    // The fetch that found it missing is the same fetch that had already
    // reported its size, and during an incident that figure is the answer to
    // the only question worth asking about a key that is no longer there.
    assert!(
        row.contains("1.1 MB") || row.contains("1.1MB"),
        "last-known size is kept: {row}"
    );
    // But not the TTL: "expires in 12m" is a claim about a key that is not
    // there to expire, where "it held 1.1 MB" stays true after the deletion.
    assert!(
        !row.contains("12m"),
        "a gone key must not count down toward an expiry that cannot happen: {row}"
    );
}

#[test]
fn a_deleted_row_shifts_nothing_around_it() {
    let before = draw(&browsing(), 130, 26);
    let after = draw(&with_a_gone_key(), 130, 26);

    let column_of = |frame: &str, needle: &str| -> Vec<usize> {
        frame
            .lines()
            .filter_map(|l| l.find(needle).map(|byte_idx| l[..byte_idx].chars().count()))
            .collect::<Vec<_>>()
    };
    // `✕` and `●` are both one column wide but different byte lengths, so this
    // is measured in characters for the same reason the M1.4 proof above is.
    for anchor in ["feed:global:hot", "stream:orders", "cart:91af3c9d2e"] {
        assert_eq!(
            column_of(&before, anchor),
            column_of(&after, anchor),
            "{anchor} moved when a key above it was deleted"
        );
    }
}

#[test]
fn a_pending_cell_is_visibly_waiting_rather_than_blank() {
    // Blank would read as "there is nothing here", which is a different claim.
    let frame = draw(&pending(), 130, 26);
    assert!(frame.contains('·'), "pending cells must show a placeholder");
    assert!(
        !frame.contains("∞"),
        "nothing is known yet, so no TTL facts"
    );

    let filled = draw(&browsing(), 130, 26);
    assert!(
        filled.contains('∞'),
        "a key with no expiry states that fact"
    );
}

/// DESIGN §2: as the terminal narrows the target is truncated from the left,
/// but the Environment and the Source are never sacrificed.
///
/// That readout is the entire mitigation for resolving a Connection silently
/// (ADR-0001), so it losing a race with the width is not a cosmetic bug.
#[test]
fn narrowing_never_costs_the_environment_or_the_source() {
    let state = State {
        connection: Connection {
            target: "redis-primary.eu-west-1.internal:6379/0".into(),
            environment: Environment::Prod,
            source: Source::Profile("prod".into()),
            topology: None,
        },
        link: up(Tk::Armed),
        read_only: Some(ReadOnlyReason::Environment),
        ..browsing()
    };

    for width in [70u16, 80, 100, 119, 130, 200] {
        let title = draw(&state, width, 26).lines().next().unwrap().to_string();
        assert!(
            title.contains("prod"),
            "Environment lost at {width}: {title}"
        );
        assert!(
            title.contains("from profile prod"),
            "Source lost at {width}: {title}"
        );
        assert!(
            title.contains("READ-ONLY"),
            "the safety readout was overwritten at {width}: {title}"
        );
        assert_eq!(
            title.chars().count(),
            width as usize,
            "the title bar must fill exactly the width at {width}"
        );
    }
}

// ── M1.5 / M1.6 / M1.7 — filter, tree, sort ─────────────────────────────────

use redis_pane_core::state::{FilterMode, SortBy};

fn many_keys() -> State {
    let mut keys = LoadedSet::default();
    let rows: &[(&str, KeyKind, i32, u32)] = &[
        ("user:8812:cart", KeyKind::ZSet, 720, 412),
        ("user:8812:profile", KeyKind::Json, TTL_NONE, 880),
        ("user:8812:session", KeyKind::Hash, 2_537, 2_150),
        ("user:8813:session", KeyKind::Hash, 3_400, 1_980),
        ("cart:91af3c9d2e", KeyKind::ZSet, 720, 1_153_434),
        ("feed:global:hot", KeyKind::List, TTL_NONE, 64_512),
    ];
    for (i, (name, kind, ttl, size)) in rows.iter().enumerate() {
        keys.push(name.as_bytes());
        keys.set_kind(i, *kind);
        keys.set_ttl(i, *ttl, 74); // matches CLOCK below
        keys.set_size(i, *size);
    }
    let mut state = State {
        keys,
        link: up(Tk::Armed),
        ..base()
    };
    state.rebuild_list();
    state
}

#[test]
fn golden_filtered_list() {
    let mut state = many_keys();
    state.list.filter = "user:*:session".into();
    state.rebuild_list();
    assert_golden("browser_filtered", &draw(&state, 130, 20));
}

#[test]
fn golden_tree_mode() {
    let mut state = many_keys();
    state.tree_mode = true;
    state.rebuild_list();
    assert_golden("browser_tree", &draw(&state, 130, 20));
}

#[test]
fn golden_sorted_by_size_partially_known() {
    let mut state = many_keys();
    // Two rows never got their size, which is the ordinary case while a list is
    // still filling.
    state.keys.set_size(0, u32::MAX - 1);
    let mut keys = LoadedSet::default();
    for (i, name) in [
        "user:8812:cart",
        "user:8812:profile",
        "unknown:a",
        "unknown:b",
    ]
    .iter()
    .enumerate()
    {
        keys.push(name.as_bytes());
        if i < 2 {
            keys.set_kind(i, KeyKind::ZSet);
            keys.set_size(i, [412u32, 880][i]);
            keys.set_ttl(i, 720, 74); // matches CLOCK below
        }
    }
    state.keys = keys;
    state.list.sort = SortBy::Size;
    state.rebuild_list();
    assert_golden("browser_sorted_partial", &draw(&state, 130, 20));
}

#[test]
fn the_filter_line_states_how_much_it_matched() {
    let mut state = many_keys();
    state.list.filter = "user:*".into();
    state.rebuild_list();
    let frame = draw(&state, 130, 20);
    assert!(frame.contains("/ user:*"), "{frame}");
    assert!(
        frame.contains("4 of 6"),
        "the reader is told the scope: {frame}"
    );
}

#[test]
fn a_partial_sort_says_so_on_screen() {
    // R2.5: ordering what arrived is fine; not saying so is not.
    let mut state = many_keys();
    let mut keys = LoadedSet::default();
    keys.push(b"known");
    keys.set_size(0, 100);
    keys.push(b"unknown");
    state.keys = keys;
    state.list.sort = SortBy::Size;
    state.rebuild_list();

    let frame = draw(&state, 130, 20);
    assert!(
        frame.contains("1 of 2 known"),
        "a partial sort must state its scope: {frame}"
    );
}

#[test]
fn tree_mode_shows_group_counts_and_leaf_names_only() {
    let mut state = many_keys();
    state.tree_mode = true;
    state.rebuild_list();
    let frame = draw(&state, 130, 20);
    assert!(frame.contains("▾ user:"), "{frame}");
    assert!(frame.contains("▾ 8812:"), "nested groups fold too: {frame}");
    // A leaf under `user:8812:` shows as `session`, not the whole path — the
    // ancestors are already on screen above it.
    // Leaf rows now carry a type-coloured dot ahead of the name (the UI task
    // this docstring predates), so the line starts with the dot, not the text.
    assert!(
        frame.lines().any(|l| l.trim_start().contains("● session")),
        "{frame}"
    );
}

#[test]
fn a_long_group_name_truncates_instead_of_overlapping_the_metadata_columns() {
    let mut keys = LoadedSet::default();
    let long_prefix = "a".repeat(100);
    let rows: &[(&str, KeyKind, i32, u32)] = &[
        ("short:one", KeyKind::Hash, 720, 412),
        ("short:two", KeyKind::Hash, 720, 412),
    ];
    let long_name = format!("{long_prefix}:session");
    for (i, (name, kind, ttl, size)) in rows.iter().enumerate() {
        keys.push(name.as_bytes());
        keys.set_kind(i, *kind);
        keys.set_ttl(i, *ttl, 74);
        keys.set_size(i, *size);
    }
    keys.push(long_name.as_bytes());
    keys.set_kind(2, KeyKind::Hash);
    keys.set_ttl(2, 720, 74);
    keys.set_size(2, 412);
    let mut state = State {
        keys,
        link: up(Tk::Armed),
        ..base()
    };
    state.tree_mode = true;
    state.rebuild_list();

    let frame = draw(&state, 80, 20);
    let group_line = frame
        .lines()
        .find(|l| l.contains('▾') && l.contains('a'))
        .unwrap_or_else(|| panic!("no group row for the long prefix: {frame}"));
    assert!(
        group_line.contains('…'),
        "a name wider than the column must be truncated, not overrun: {group_line}"
    );
    // The row's own count column, drawn after the name, must land at its usual
    // right-aligned position rather than being swallowed by an unbounded name.
    assert!(
        group_line.trim_end().ends_with("1 │") || group_line.trim_end().ends_with('1'),
        "the descendants count must survive at the end of the row: {group_line:?}"
    );
}

#[test]
fn filtering_narrows_the_tree_as_well_as_the_flat_list() {
    let mut state = many_keys();
    state.tree_mode = true;
    state.list.filter = "cart".into();
    state.rebuild_list();
    let frame = draw(&state, 130, 20);
    assert!(!frame.contains("session"), "filtered out: {frame}");
    assert!(frame.contains("cart"), "{frame}");
}

#[test]
fn fuzzy_mode_matches_characters_in_order() {
    let mut state = many_keys();
    state.list.mode = FilterMode::Fuzzy;
    // `u88ses` would match 8812 *and* 8813 — fuzzy is deliberately generous.
    state.list.filter = "u8812ses".into();
    state.rebuild_list();
    assert_eq!(state.list.len(), 1);
    assert_eq!(
        state
            .keys
            .name_str(state.list.index_at(0).unwrap())
            .unwrap(),
        "user:8812:session"
    );
}

// ── M1.8 / M1.9 — one frame, eight bodies ───────────────────────────────────

use redis_pane_core::state::OpenKey;
use redis_pane_core::state::value::{
    BinaryValue, IndexedValue, JsonValue, MemberValue, PairValue, ScoredValue, StreamValue,
    StringValue, Value,
};

/// A key open, with the cursor on its row — the Attached case, which is what
/// these viewer fixtures are about.
///
/// It opens *the row that actually holds this name*, adding it to the Loaded
/// set if it is not already there. The earlier version opened index 0 whatever
/// the name was, which was invisible while nothing tied the panes together and
/// is a self-contradicting frame now that something does: a header naming one
/// key over a mark pointing at another is precisely the confusion these fixtures
/// exist to catch.
fn opened(name: &str, value: Value, ttl: i32) -> State {
    let mut state = many_keys();
    let index = (0..state.keys.len())
        .find(|&i| state.keys.name_str(i).as_deref() == Some(name))
        .unwrap_or_else(|| {
            state.keys.push(name.as_bytes());
            state.keys.len() - 1
        });
    state.open = Some(OpenKey::new(
        Some(index),
        name.into(),
        value,
        ttl,
        2_150,
        60_000,
    ));
    state.rebuild_list();
    state.view.selected = state.open.as_ref().and_then(|open| open.row).unwrap_or(0);
    state
}

fn hash_value() -> Value {
    Value::Hash(PairValue {
        pairs: vec![
            ("id".into(), "8812".into()),
            ("device".into(), "ios/17.2".into()),
            ("region".into(), "eu-west-1".into()),
            ("plan".into(), "pro".into()),
            ("locale".into(), "fr-FR".into()),
        ],
        total: 5,
    })
}

#[test]
fn golden_viewer_hash() {
    assert_golden(
        "viewer_hash",
        &draw(&opened("user:8812:session", hash_value(), 2_537), 130, 22),
    );
}

#[test]
fn golden_viewer_cursor_active() {
    // `Enter` (`Action::EnterValueCursor`) highlights a row inside the value
    // the same way the keys pane highlights its own selected row. `draw()`
    // renders monochrome text only, so this fixture pins the row content and
    // layout; the style itself is proved by the two tests below, mirroring
    // `the_selected_row_carries_a_background_all_the_way_across_not_just_on_the_name`.
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1;
    let frame = draw(&state, 130, 22);
    assert!(
        frame.contains("device"),
        "the highlighted row's field: {frame}"
    );
    assert_golden("viewer_cursor_active", &frame);
}

#[test]
fn the_value_cursor_row_carries_the_selected_background_all_the_way_across() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // the "device" row
    let theme = Theme::new(ColorDepth::TrueColor);
    let frame = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 130, 22));
    let y = row_of(&frame, "device");

    let selected_bg = theme.style(Token::Selected).bg;
    assert!(selected_bg.is_some());

    let value_pane = redis_pane_core::render::layout::layout(
        Rect::new(0, 0, 130, 22),
        redis_pane_core::render::layout::Pane::Value,
        0,
    )
    .value
    .unwrap();

    // The value pane reserves a one-column margin on each side everywhere
    // (the name/header text starts at `area.x + 1`, right-aligned status
    // text spans `area.width - 1`) — the highlight matches that, rather than
    // touching the frame's outer border column.
    let mut gaps = Vec::new();
    for x in value_pane.x + 1..value_pane.x + value_pane.width - 1 {
        match frame.cell((x, y)) {
            Some(cell) if cell.style().bg == selected_bg => {}
            other => gaps.push((x, other.map(|c| c.style().bg))),
        }
    }
    assert!(
        gaps.is_empty(),
        "the cursor row's highlight has gaps: {gaps:?}"
    );
}

#[test]
fn a_row_is_only_highlighted_while_the_cursor_is_active_and_on_it() {
    let state = opened("user:8812:session", hash_value(), 2_537);
    let theme = Theme::new(ColorDepth::TrueColor);
    let frame = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 130, 22));
    let y = row_of(&frame, "device");
    let selected_bg = theme.style(Token::Selected).bg;

    let value_pane = redis_pane_core::render::layout::layout(
        Rect::new(0, 0, 130, 22),
        redis_pane_core::render::layout::Pane::Value,
        0,
    )
    .value
    .unwrap();

    assert!(
        (value_pane.x + 1..value_pane.x + value_pane.width)
            .all(|x| frame.cell((x, y)).map(|c| c.style().bg) != Some(selected_bg)),
        "cursor_active is false, so nothing should be highlighted"
    );
}

#[test]
fn golden_viewer_zset() {
    let v = Value::ZSet(ScoredValue {
        entries: vec![
            ("sku-100".into(), 1.0),
            ("sku-221".into(), 2.0),
            ("sku-874".into(), 3.5),
        ],
        total: 3,
    });
    assert_golden(
        "viewer_zset",
        &draw(&opened("user:8812:cart", v, 720), 130, 22),
    );
}

#[test]
fn golden_viewer_stream() {
    // Newest first (XREVRANGE), and close enough to CLOCK (74_000) for the AGE
    // column to show something real rather than saturating to "just now" —
    // real stream IDs carry genuine epoch millis, which this fixed test clock
    // deliberately does not use anywhere else in this file.
    let v = Value::Stream(StreamValue {
        entries: vec![
            (
                "72000-0".into(),
                vec![
                    ("order".into(), "1001".into()),
                    ("amount".into(), "17".into()),
                ],
            ),
            (
                "10000-0".into(),
                vec![
                    ("order".into(), "1000".into()),
                    ("amount".into(), "42".into()),
                ],
            ),
        ],
        total: 500,
    });
    assert_golden(
        "viewer_stream",
        &draw(&opened("stream:orders", v, TTL_NONE), 130, 22),
    );
}

// ── focus is visible, because `r` means different things per pane ───────────

#[test]
fn which_pane_has_focus_is_visible_without_pressing_anything() {
    use redis_pane_core::render::layout::Pane;
    let keys_focused = State {
        focus: Pane::Keys,
        ..opened("user:8812:session", hash_value(), 2_537)
    };
    let value_focused = State {
        focus: Pane::Value,
        ..opened("user:8812:session", hash_value(), 2_537)
    };
    let a = render::to_golden(&render::frame(
        &keys_focused,
        &Theme::new(ColorDepth::TrueColor),
        &CLOCK,
        Rect::new(0, 0, 130, 22),
    ));
    let b = render::to_golden(&render::frame(
        &value_focused,
        &Theme::new(ColorDepth::TrueColor),
        &CLOCK,
        Rect::new(0, 0, 130, 22),
    ));
    let text_of = |g: &str| g.split("--- styles ---").next().unwrap().to_string();
    assert_eq!(
        text_of(&a),
        text_of(&b),
        "focus is emphasis, not content — no glyph, no reflow, no width change"
    );
    assert_ne!(a, b, "but it must be visible in the styles");
}

#[test]
fn focus_survives_the_loss_of_colour() {
    // It says which pane `r` will act on, so it is information rather than
    // decoration, and this module's rule is that losing colour loses emphasis
    // and never information. Muted is DIM in monochrome where Text is plain.
    let theme = Theme::new(ColorDepth::Monochrome);
    assert_ne!(
        theme.style(redis_pane_core::theme::Token::Text),
        theme.style(redis_pane_core::theme::Token::Muted),
        "the focused and unfocused pane headers would be indistinguishable"
    );
}

// ── the header must not pass a window off as the whole value ────────────────

#[test]
fn a_windowed_value_says_how_much_of_it_is_on_screen() {
    // `XLEN` says 500; the read brought back 2. Stating only the first turns
    // "the newest 2 of these" into "this is all of it" — the scan-cap defect
    // one level down, and the reason a search over these rows was cut rather
    // than built on top of a header that lies about its own scope.
    let v = Value::Stream(StreamValue {
        entries: vec![("72000-0".into(), vec![("order".into(), "1001".into())])],
        total: 500,
    });
    let frame = draw(&opened("stream:orders", v, TTL_NONE), 130, 22);
    let header = frame
        .lines()
        .find(|l| l.contains("stream ·"))
        .expect("the viewer header names the type");
    assert!(header.contains("500 entries"), "the real length: {header}");
    assert!(
        header.contains("1 shown"),
        "and what is on screen: {header}"
    );
}

#[test]
fn a_windowed_hash_says_how_much_of_it_is_on_screen() {
    // `HLEN` says 1500; `HSCAN` brought back 1 field for this fixture. Hash
    // was, with Set, the last type that stayed unbounded (`HGETALL` pulling
    // the whole thing) after List/ZSet/Stream were windowed — same defect,
    // same fix, same header rule.
    let v = Value::Hash(PairValue {
        pairs: vec![("f1".into(), "v1".into())],
        total: 1_500,
    });
    let frame = draw(&opened("bighash", v, TTL_NONE), 130, 22);
    let header = frame
        .lines()
        .find(|l| l.contains("hash ·"))
        .expect("the viewer header names the type");
    assert!(header.contains("1500 fields"), "the real length: {header}");
    assert!(
        header.contains("1 shown"),
        "and what is on screen: {header}"
    );
}

#[test]
fn a_value_fetched_whole_says_nothing_extra() {
    // A hash comes back complete, so there is no window to disclose and the
    // header must not grow a phrase that would read as a caveat where none
    // applies.
    let frame = draw(&opened("user:8812:session", hash_value(), 2_537), 130, 22);
    let header = frame
        .lines()
        .find(|l| l.contains("hash ·"))
        .expect("the viewer header names the type");
    assert!(!header.contains("shown"), "nothing is withheld: {header}");
}

#[test]
fn golden_viewer_binary() {
    let v = Value::Binary(BinaryValue {
        bytes: (0u8..48).collect(),
    });
    assert_golden(
        "viewer_binary",
        &draw(&opened("blob:thumb", v, TTL_NONE), 130, 22),
    );
}

#[test]
fn golden_viewer_json() {
    let v = Value::Json(JsonValue::parse(
        r#"{"newnav":true,"ab_checkout":"B","rollout":0.25}"#,
    ));
    assert_golden(
        "viewer_json",
        &draw(&opened("config:feature-flags", v, TTL_NONE), 130, 22),
    );
}

/// R3.1's proof, stated as a frame comparison rather than a claim: the header
/// and footer are byte-identical across types, so navigation can be shared.
#[test]
fn the_frame_around_the_body_is_identical_for_every_type() {
    let cases = vec![
        Value::Str(StringValue::new("hello world", 60)),
        hash_value(),
        Value::List(IndexedValue {
            items: vec!["x".into()],
            total: 1,
        }),
        Value::Set(MemberValue {
            members: vec!["m".into()],
            total: 1,
        }),
        Value::ZSet(ScoredValue {
            entries: vec![("m".into(), 1.0)],
            total: 1,
        }),
        Value::Stream(StreamValue {
            entries: vec![],
            total: 0,
        }),
        Value::Json(JsonValue::parse("{}")),
        Value::Binary(BinaryValue { bytes: vec![1] }),
    ];

    let mut first_header: Option<String> = None;
    for value in cases {
        let frame = draw(&opened("k", value, 600), 130, 22);
        let lines: Vec<&str> = frame.lines().collect();
        // Row 0 is the title bar; the value pane's key name and ttl rows are
        // shared chrome and must not vary with the type.
        let key_row = lines[2].split('│').nth(1).unwrap_or("").to_string();
        let ttl_row = lines[4].split('│').nth(1).unwrap_or("").to_string();
        let combined = format!("{key_row}|{ttl_row}");
        match &first_header {
            None => first_header = Some(combined),
            Some(expected) => assert_eq!(
                &combined, expected,
                "the shared frame differed between types"
            ),
        }
    }
}

// ── M1.10 — the liveness states, on screen ──────────────────────────────────

#[test]
fn golden_viewer_update_held_while_scrolled() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.offset = 2;
    open.at_rest = false;
    open.absorb(hash_value(), 2_400, 2_200, 72_000);
    assert_golden("viewer_changed_held", &draw(&state, 130, 22));
}

#[test]
fn golden_viewer_deleted() {
    let mut state = opened("lock:checkout:8812", hash_value(), 12);
    state.open.as_mut().unwrap().deleted_at_ms = Some(71_000);
    // The row learns it too. `Msg::ValueGone` does both together (958b311); a
    // fixture that badged only the Viewer would picture the two panes
    // disagreeing about one key, which is the state that commit removed.
    let index = state.open.as_ref().unwrap().index.unwrap();
    state.keys.set_gone(index);
    assert_golden("viewer_deleted", &draw(&state, 130, 22));
}

#[test]
fn an_update_at_rest_lands_with_no_keypress() {
    // The whole point of ADR-0006, asserted at the frame level.
    let mut state = opened("k", hash_value(), 600);
    let before = draw(&state, 130, 22);

    let changed = Value::Hash(PairValue {
        pairs: vec![("plan".into(), "enterprise".into())],
        total: 1,
    });
    state
        .open
        .as_mut()
        .unwrap()
        .absorb(changed, 600, 2_150, 61_000);

    let after = draw(&state, 130, 22);
    assert_ne!(before, after, "the new value must be on screen already");
    assert!(after.contains("enterprise"));
    assert!(
        !after.contains("r to load"),
        "nothing was asked of the reader"
    );
}

/// ADR-0006's founding complaint, at the frame level: a Refetch that finds
/// nothing must not render a frame identical to one where no read happened.
#[test]
fn a_refetch_that_found_nothing_still_changes_the_frame() {
    let mut state = opened("k", hash_value(), 600);
    let before = draw(&state, 130, 22);

    // The same value comes back — the ordinary case for a key nobody is
    // writing to, and the case that used to be indistinguishable from the
    // reply being dropped as superseded, or failing, or never being sent.
    state
        .open
        .as_mut()
        .unwrap()
        .absorb(hash_value(), 600, 2_150, 73_000);

    let after = draw(&state, 130, 22);
    assert_ne!(
        before, after,
        "the reader has to be able to tell that a read happened"
    );
    assert!(after.contains("unchanged"), "{after}");
}

#[test]
fn golden_viewer_read_outcomes() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state
        .open
        .as_mut()
        .unwrap()
        .absorb(hash_value(), 2_537, 2_150, 73_000);
    assert_golden("viewer_unchanged", &draw(&state, 130, 22));

    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let changed = Value::Hash(PairValue {
        pairs: vec![("plan".into(), "enterprise".into())],
        total: 1,
    });
    state
        .open
        .as_mut()
        .unwrap()
        .absorb(changed, 2_537, 2_150, 73_000);
    assert_golden("viewer_updated", &draw(&state, 130, 22));
}

#[test]
fn an_update_while_scrolled_is_announced_and_offers_the_effective_key() {
    let mut state = opened("k", hash_value(), 600);
    let open = state.open.as_mut().unwrap();
    open.offset = 3;
    open.at_rest = false;
    open.absorb(
        Value::Hash(PairValue {
            pairs: vec![("plan".into(), "enterprise".into())],
            total: 1,
        }),
        600,
        2_150,
        // CLOCK is 74_000, so this arrived two seconds ago.
        72_000,
    );

    let frame = draw(&state, 130, 22);
    assert!(frame.contains("changed 2s ago"), "{frame}");
    assert!(frame.contains("r to load"), "{frame}");
    assert!(
        !frame.contains("enterprise"),
        "nothing moved under the reader"
    );
}

#[test]
fn a_deleted_key_keeps_its_value_on_screen() {
    let mut state = opened("k", hash_value(), 600);
    state.open.as_mut().unwrap().deleted_at_ms = Some(63_000);
    let frame = draw(&state, 130, 22);
    assert!(frame.contains("✕ deleted"), "{frame}");
    assert!(frame.contains("ios/17.2"), "the evidence survives: {frame}");
}

#[test]
fn the_ttl_counts_down_between_frames_without_any_fetch() {
    let state = opened("k", hash_value(), 600);
    let at_open = render::frame(
        &state,
        &Theme::new(ColorDepth::Monochrome),
        &FixedClock(60_000),
        Rect::new(0, 0, 130, 22),
    );
    let a_minute_later = render::frame(
        &state,
        &Theme::new(ColorDepth::Monochrome),
        &FixedClock(120_000),
        Rect::new(0, 0, 130, 22),
    );
    let text = |b: &ratatui::buffer::Buffer| render::to_text(b);
    assert!(text(&at_open).contains("ttl 10m"));
    assert!(
        text(&a_minute_later).contains("ttl 9m"),
        "{}",
        text(&a_minute_later)
    );
}

// ── M1.12 — copy ────────────────────────────────────────────────────────────

use redis_pane_core::keymap::{Action as Act, Keymap as Km};
use redis_pane_core::msg::KeyCode as KC;
use redis_pane_core::state::copy::{redis_cli_command, value_text};
use redis_pane_core::{Command, Msg, update};

fn press(state: State, c: char) -> (State, Vec<Command>) {
    update(state, Msg::Key(KeyPress::plain(KC::Char(c))))
}

/// Focused on the Keys pane, `y` copies the key name — no mnemonic, no chord.
#[test]
fn c_copies_the_key_name_when_the_keys_pane_is_focused() {
    let state = opened("user:8812:session", hash_value(), 600);
    assert!(state.keys_pane_focused(), "opening does not move focus");
    let (_, cmds) = press(state, 'c');
    match cmds.first() {
        Some(Command::CopyToClipboard { text, label }) => {
            assert_eq!(text, "user:8812:session");
            assert_eq!(*label, "key");
        }
        other => panic!("expected a copy, got {other:?}"),
    }
}

/// Focused on the Viewer, the same `y` copies the value instead — the pane
/// under the reader's cursor decides what "it" refers to.
#[test]
fn c_copies_the_value_when_the_viewer_is_focused() {
    let mut state = opened("k", hash_value(), 600);
    state.focus = redis_pane_core::render::layout::Pane::Value;
    let (_, cmds) = press(state, 'c');
    match cmds.first() {
        Some(Command::CopyToClipboard { text, .. }) => {
            assert_eq!(
                text.lines().count(),
                5,
                "all five fields, not the visible ones"
            );
            assert!(text.contains("device\tios/17.2"));
        }
        other => panic!("expected a copy, got {other:?}"),
    }
}

#[test]
fn shift_c_copies_a_command_that_would_actually_run() {
    let state = opened("user:8812:session", hash_value(), 600);
    let (_, cmds) = press(state, 'C');
    match cmds.first() {
        Some(Command::CopyToClipboard { text, .. }) => {
            assert_eq!(
                text,
                "redis-cli -h cache-01 -p 6379 -n 0 HGETALL user:8812:session"
            );
        }
        other => panic!("expected a copy, got {other:?}"),
    }
}

#[test]
fn the_key_name_is_copyable_from_the_list_with_nothing_open() {
    let mut state = many_keys();
    state.open = None;
    let (_, cmds) = press(state, 'c');
    assert!(
        matches!(cmds.first(), Some(Command::CopyToClipboard { .. })),
        "a key name needs no open value"
    );
}

#[test]
fn copying_a_value_with_nothing_open_says_so_instead_of_copying_nothing() {
    let mut state = many_keys();
    state.open = None;
    state.focus = redis_pane_core::render::layout::Pane::Value;
    let (state, cmds) = press(state, 'c');
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::CopyToClipboard { .. })),
        "nothing was put on the clipboard"
    );

    // The notice goes out for the shell to date. This test used to read it at
    // clock 0 and pass — the one clock reading at which the defect it was
    // guarding is invisible. The core built the notice with `at_ms: 0`, and
    // `notice_now` shows a notice for 2.5s against a clock reading epoch
    // milliseconds, so in the running app the message could never appear:
    // `y` with nothing open did nothing at all, forever.
    let Some(Command::Notify { text }) = cmds.first().cloned() else {
        panic!("expected a notice, got {cmds:?}");
    };
    let (state, _) = update(
        state,
        Msg::Noticed {
            text,
            at_ms: 73_000,
        },
    );
    assert_eq!(state.notice_now(73_100), Some("nothing open to copy"));
}

/// The clipboard shows no seams: 500 rows of a 12,000-item list look exactly
/// like a complete copy once pasted. The Viewer header already states this
/// about the value; the confirmation has to state it about the copy.
#[test]
fn copying_a_windowed_value_says_how_much_it_took() {
    let windowed = Value::List(IndexedValue {
        items: (0..500).map(|i| format!("item-{i}").into_bytes()).collect(),
        total: 12_000,
    });
    let mut state = opened("feed:global:hot", windowed, 600);
    state.focus = redis_pane_core::render::layout::Pane::Value;
    let (_, cmds) = press(state, 'c');

    let Some(Command::CopyToClipboard { label, text }) = cmds.first() else {
        panic!("expected a copy, got {cmds:?}");
    };
    assert_eq!(
        label, "value (500 of 12000 items)",
        "a partial copy that calls itself `value` is a trap"
    );
    assert_eq!(text.lines().count(), 500, "and it really is the window");
}

/// A whole value says nothing extra — the qualifier appears only where there
/// is something to qualify.
#[test]
fn copying_a_complete_value_stays_quiet_about_it() {
    let mut state = opened("user:8812:session", hash_value(), 600);
    state.focus = redis_pane_core::render::layout::Pane::Value;
    let (_, cmds) = press(state, 'c');
    let Some(Command::CopyToClipboard { label, .. }) = cmds.first() else {
        panic!("expected a copy");
    };
    assert_eq!(label, "value");
}

#[test]
fn the_confirmation_fades_on_its_own() {
    // A notice you must dismiss is a modal dialog wearing a smaller hat.
    let (state, _) = update(
        opened("k", hash_value(), 600),
        Msg::Copied {
            label: "key".into(),
            at_ms: 70_000,
        },
    );
    assert_eq!(state.notice_now(70_500), Some("copied key"));
    assert_eq!(state.notice_now(74_000), None, "gone by 4 seconds");
}

#[test]
fn golden_copy_notice() {
    let (state, _) = update(
        opened("user:8812:session", hash_value(), 2_537),
        Msg::Copied {
            label: "redis-cli command".into(),
            at_ms: 73_000,
        },
    );
    assert_golden("copy_notice", &draw(&state, 130, 22));
}

#[test]
fn the_copy_binding_appears_in_the_help_overlay() {
    // Keybindings are data, so the overlay follows automatically (R7.5).
    // `base()` has nothing open and Keys focused, so `help::here`'s `copy`
    // row is the keys pane's (Copy is focus-dependent, not scoped to a
    // value type) — this is `help_lines`'s replacement (M3: the overlay is
    // now contextual, not a flat list of every binding).
    let mut keymap = Km::default();
    assert!(keymap.hint(Act::Copy).is_some());
    keymap.bind(Act::Copy, KeyPress::ctrl(KC::Char('y')));
    let state = State { keymap, ..base() };
    assert!(
        help::here(&state, help::context(&state))
            .iter()
            .any(|r| r.keys == "⌃Y")
    );
}

#[test]
fn the_command_uses_the_target_the_title_bar_is_showing() {
    // A copied command that points at a different server than the one on
    // screen would be actively dangerous.
    let state = opened("k", hash_value(), 600);
    let open = state.open.as_ref().unwrap();
    let cmd = redis_cli_command(
        &state.connection.target,
        &open.name,
        open.value.as_ref().unwrap(),
        false,
    );
    assert!(cmd.contains("cache-01"), "{cmd}");
    assert_eq!(state.connection.target, "cache-01:6379/0");
}

#[test]
fn copying_a_value_is_not_affected_by_where_the_viewer_is_scrolled() {
    let mut state = opened("k", hash_value(), 600);
    state.open.as_mut().unwrap().offset = 3;
    let full = value_text(state.open.as_ref().unwrap().value.as_ref().unwrap(), 0);
    assert_eq!(full.lines().count(), 5);
}

// ── severity-4: editing indicator in the value pane header ──────────────────

#[test]
fn golden_viewer_editing_with_nothing_pending() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    // Mid-edit with no buffer on screen: the header alone says so.
    state.open.as_mut().unwrap().edit = redis_pane_core::state::EditPhase::Saving;
    assert_golden("viewer_editing", &draw(&state, 130, 22));
}

#[test]
fn editing_is_visibly_distinct_from_plain_live_in_monochrome_too() {
    // Colour is never the only carrier of meaning (DESIGN §5): the word
    // "editing" must appear even with hue gone entirely.
    let mut state = opened("k", hash_value(), 600);
    // Mid-edit with no buffer on screen: the header alone says so.
    state.open.as_mut().unwrap().edit = redis_pane_core::state::EditPhase::Saving;
    let mono = render::frame(
        &state,
        &Theme::new(ColorDepth::Monochrome),
        &CLOCK,
        Rect::new(0, 0, 130, 22),
    );
    assert!(render::to_text(&mono).contains("editing"));
}

// ── M2 task 4 rework: the inline value editor (ADR-0014) ────────────────────

use redis_pane_core::state::EditBuffer;

#[test]
fn golden_editor_open_on_a_string() {
    let mut state = opened(
        "user:8812:session",
        Value::Str(StringValue::new("v1", 65)),
        600,
    );
    let editor = EditBuffer::from_value(
        &Value::Str(StringValue::new("hello, this is the unsaved buffer", 65)),
        0,
    )
    .unwrap();
    let open = state.open.as_mut().unwrap();
    open.begin_edit(editor);
    assert_golden("editor_open_on_a_string", &draw(&state, 130, 22));
}

#[test]
fn golden_editor_json_invalid_shows_json_cross() {
    let mut state = opened(
        "user:8812:session",
        Value::Json(JsonValue::parse("{\"a\":1}")),
        600,
    );
    let mut editor =
        EditBuffer::from_value(&Value::Json(JsonValue::parse("{\"a\":1}")), 0).unwrap();
    editor.insert_char('x'); // breaks the JSON
    let open = state.open.as_mut().unwrap();
    open.begin_edit(editor);
    let frame = draw(&state, 130, 22);
    assert!(
        frame.contains("json ✗"),
        "must show the invalid marker:\n{frame}"
    );
    assert_golden("editor_json_invalid", &frame);
}

#[test]
fn golden_editor_wraps_a_long_line() {
    let mut state = opened(
        "user:8812:session",
        Value::Str(StringValue::new("v1", 65)),
        600,
    );
    let long = "word ".repeat(60);
    let editor = EditBuffer::from_value(&Value::Str(StringValue::new(&long, 65)), 0).unwrap();
    let open = state.open.as_mut().unwrap();
    open.begin_edit(editor);
    assert_golden("editor_wraps_a_long_line", &draw(&state, 80, 22));
}

#[test]
fn golden_editor_hint_bar() {
    let mut state = opened(
        "user:8812:session",
        Value::Str(StringValue::new("v1", 65)),
        600,
    );
    let editor = EditBuffer::from_value(&Value::Str(StringValue::new("v1", 65)), 0).unwrap();
    let open = state.open.as_mut().unwrap();
    open.begin_edit(editor);
    // `F1 help` is pinned last now (decision 4/5, M3): every context keeps
    // help discoverable, including the editor, where `?` no longer opens it.
    assert_eq!(
        hint_bar(&state, state.cols),
        "⌃S stage   ⌃Z undo   Esc cancel   F1 help"
    );
}

#[test]
fn golden_confirm_diff_after_staging_an_edit() {
    let mut state = opened(
        "user:8812:session",
        Value::Str(StringValue::new("old value", 65)),
        600,
    );
    // `e` is focus-gated (G, PLAN M2 task 6 follow-up); `opened()` leaves
    // focus on the keys pane.
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("!");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(state.confirm.is_some());
    assert_golden("editor_confirm_diff_after_staging", &draw(&state, 130, 22));
}

// ── PLAN M2 task 6: editing Hash fields (D1–D4, ADR-0015) ───────────────────

use redis_pane_core::state::PendingMutation;

#[test]
fn golden_editing_an_existing_field_shows_it_read_only_above_the_active_value() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "device"
    open.begin_edit(EditBuffer::for_hash_field(b"device", b"ios/17.2").unwrap());
    assert_golden("hash_form_edit_field", &draw(&state, 130, 22));
}

#[test]
fn golden_hash_add_form_name_part_shows_the_placeholder_on_value() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_hash_field());
    let editor = open.typing_mut().unwrap();
    editor.name_push('n');
    editor.name_push('e');
    editor.name_push('w');
    editor.name_push('_');
    editor.name_push('f');
    editor.name_push('i');
    assert_golden("hash_form_add_name", &draw(&state, 130, 22));
}

#[test]
fn golden_hash_add_form_duplicate_name_shows_the_warning() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_hash_field());
    let editor = open.typing_mut().unwrap();
    // "device" is one of `hash_value()`'s fields — a shown duplicate.
    for c in "device".chars() {
        editor.name_push(c);
    }
    assert_golden("hash_form_add_duplicate", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_set_hash_field_shows_the_effective_command_and_its_guard() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    // `e` is focus-gated (G, PLAN M2 task 6 follow-up); `opened()` leaves
    // focus on the keys pane.
    state.focus = Pane::Value;
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "device": a plain string, to keep this fixture about
    // the guard line and the diff rather than the JSON-parses warning
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("x");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(state.confirm.is_some());
    assert_golden("confirm_set_hash_field", &draw(&state, 130, 22));
}

#[test]
fn golden_hash_add_form_value_part_shows_field_above_the_active_editor() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let state = "country".chars().fold(state, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("fr");
    assert_golden("hash_form_add_value", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_add_hash_field_shows_the_guard_and_only_a_plus_side() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let state = "country".chars().fold(state, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("fr");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(state.confirm.is_some());
    assert_golden("confirm_add_hash_field", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_hash_field_warns_when_it_is_the_last_field() {
    let mut state = opened(
        "user:8812:session",
        Value::Hash(PairValue {
            pairs: vec![("only".into(), "v".into())],
            total: 1,
        }),
        600,
    );
    // `d` is focus-dependent (D4): without this, `opened()`'s default focus
    // (the keys pane) makes `d` stage `DeleteKey`, not `DeleteHashField` — the
    // HDEL dialog and its last-field warning never appear at all, silently.
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteHashField { .. })
    ));
    assert_golden(
        "confirm_delete_hash_field_last_field",
        &draw(&state, 130, 22),
    );
}

#[test]
fn golden_hint_bar_names_all_three_hash_field_actions_with_a_cursor_active() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    // The bar is contextual now (M3): more than three rows show, and the
    // labels name the target (decision 6) rather than the bare verb — the
    // meaning this test pins is still all three actions being on offer.
    let hint = hint_bar(&state, state.cols);
    assert!(hint.contains("e edit field"), "{hint}");
    assert!(hint.contains("a add field"), "{hint}");
    assert!(hint.contains("d delete field"), "{hint}");
}

/// D4: `Tab` (`Action::CyclePane`) can move focus back to the keys pane
/// without clearing `cursor_active` — the Hash field hints must not survive
/// that, since `d` there stages `DeleteKey`, not `HDEL` (the bug the review
/// caught: the hint claimed `remove` for a `d` that would delete the whole
/// key).
#[test]
fn the_hash_field_hint_does_not_claim_remove_when_the_keys_pane_is_focused() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Keys;
    state.open.as_mut().unwrap().cursor_active = true;
    let hint = hint_bar(&state, state.cols);
    assert!(
        !hint.contains("remove"),
        "keys-pane `d` stages DeleteKey, not HDEL: {hint}"
    );
}

// ── PLAN M2 task 7: adding/removing Set members (D1–D5, ADR-0016) ──────────

fn set_value() -> Value {
    Value::Set(MemberValue {
        members: vec!["alpha".into(), "beta".into(), "gamma".into()],
        total: 3,
    })
}

#[test]
fn golden_set_add_form_empty_shows_the_single_capture() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_set_member());
    assert_golden("set_form_add_empty", &draw(&state, 130, 22));
}

#[test]
fn golden_set_add_form_typed_shows_the_text_in_the_single_part() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_set_member());
    let editor = open.typing_mut().unwrap();
    editor.insert_str("delta");
    assert_golden("set_form_add_typed", &draw(&state, 130, 22));
}

#[test]
fn golden_set_add_form_shown_duplicate_blocks_staging() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_set_member());
    let editor = open.typing_mut().unwrap();
    // "alpha" is one of `set_value()`'s members — a shown duplicate.
    editor.insert_str("alpha");
    assert_golden("set_form_add_duplicate", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_add_set_member_shows_the_guard_and_only_a_plus_side() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("delta");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::AddSetMember { .. })
    ));
    assert_golden("confirm_add_set_member", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_set_member_shows_the_effective_command() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    // `d` is focus-dependent (D5): without this, `opened()`'s default focus
    // (the keys pane) makes `d` stage `DeleteKey`, not `DeleteSetMember`.
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteSetMember {
            last_member: false,
            ..
        })
    ));
    assert_golden("confirm_delete_set_member", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_set_member_warns_when_it_is_the_last_member() {
    let mut state = opened(
        "user:8812:session",
        Value::Set(MemberValue {
            members: vec!["only".into()],
            total: 1,
        }),
        600,
    );
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteSetMember {
            last_member: true,
            ..
        })
    ));
    assert_golden(
        "confirm_delete_set_member_last_member",
        &draw(&state, 130, 22),
    );
}

/// D1: `e` on a Set never opens a buffer — only `a`/`d` mean anything for a
/// Set (ADR-0016), so the hint bar must not promise an `edit` mode
/// `open_editor` never opens.
#[test]
fn golden_hint_bar_names_only_add_and_remove_for_a_set_with_a_cursor_active() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let hint = hint_bar(&state, state.cols);
    assert!(hint.contains("a add member"), "{hint}");
    assert!(hint.contains("d remove member"), "{hint}");
    // "edit ttl" (`t`, M3) is on this bar too and does contain "edit" — the
    // constraint is that `e` itself never gets a row for a Set, not that
    // the word never appears anywhere on the bar.
    assert!(
        !hint.contains("e edit"),
        "D1: e always refuses on a Set, so there is no edit row at all: {hint}"
    );
}

// ── PLAN M2 task 8: editing List elements (D1–D8, ADR-0017) ────────────────

fn list_value() -> Value {
    Value::List(IndexedValue {
        items: vec!["alpha".into(), "beta".into(), "gamma".into()],
        total: 3,
    })
}

#[test]
fn golden_list_form_edit_element_shows_the_raw_value_by_index() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "beta"
    open.begin_edit(EditBuffer::list_element(1, b"beta").unwrap());
    assert_golden("list_form_edit_element", &draw(&state, 130, 22));
}

/// D6: the add form defaults to `Tail` — appending is the common case, and
/// the one that does not renumber the rows the reader is already looking at.
#[test]
fn golden_list_add_form_shows_tail_by_default() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("delta");
    assert_golden("list_form_add_tail", &draw(&state, 130, 22));
}

/// D6: `Tab` flips the end, and the form's label must say so — this frame
/// and the one above must be visibly different, not just internally
/// different, or the toggle is invisible to the reader using it.
#[test]
fn golden_list_add_form_shows_head_after_tab_toggles_it() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("delta");
    editor.toggle_list_end();
    let frame = draw(&state, 130, 22);
    assert!(
        frame.contains("head"),
        "the toggled form must say which end is active:\n{frame}"
    );
    assert_golden("list_form_add_head", &frame);
}

#[test]
fn golden_confirm_set_list_element_shows_the_effective_command_and_its_guard() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    // `e` is focus-gated (ADR-0015 D4, mirrored for Lists); `opened()` leaves
    // focus on the keys pane.
    state.focus = Pane::Value;
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "beta"
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("!");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::SetListElement { .. })
    ));
    assert_golden("confirm_set_list_element", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_add_list_element_shows_the_guard_and_only_a_plus_side() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("delta");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::AddListElement { .. })
    ));
    assert_golden("confirm_add_list_element", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_list_element_shows_the_effective_command() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    // `d` is focus-dependent (D5): without this, `opened()`'s default focus
    // (the keys pane) makes `d` stage `DeleteKey`, not `DeleteListElement`.
    state.focus = Pane::Value;
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "beta"
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteListElement {
            last_element: false,
            ..
        })
    ));
    assert_golden("confirm_delete_list_element", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_list_element_warns_when_it_is_the_last_element() {
    let mut state = opened(
        "user:8812:session",
        Value::List(IndexedValue {
            items: vec!["only".into()],
            total: 1,
        }),
        600,
    );
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteListElement {
            last_element: true,
            ..
        })
    ));
    assert_golden(
        "confirm_delete_list_element_last_element",
        &draw(&state, 130, 22),
    );
}

#[test]
fn golden_hint_bar_names_all_three_list_element_actions_with_a_cursor_active() {
    let mut state = opened("user:8812:session", list_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let hint = hint_bar(&state, state.cols);
    assert!(hint.contains("e edit element"), "{hint}");
    assert!(hint.contains("a add element"), "{hint}");
    assert!(hint.contains("d delete element"), "{hint}");
}

// ── PLAN M2 task 9: editing a ZSet's score, add/remove members (D1–D8, ADR-0018) ──

fn zset_value() -> Value {
    Value::ZSet(ScoredValue {
        entries: vec![
            ("alpha".into(), 1.0),
            ("beta".into(), 2.0),
            ("gamma".into(), 3.5),
        ],
        total: 3,
    })
}

/// Replace the score buffer's whole text, mirroring
/// `crates/core/src/update/editor.rs`'s own `retype_score` test helper:
/// [`EditBuffer::zset_score`] seeds the cursor at the *start* of the score
/// text, not the end, so a bare `Backspace` right after `e` deletes nothing
/// (phase 3's "Found while building" — a real, deliberately-unfixed defect,
/// not a mistake in this fixture). Moving to the end and clearing first is
/// what any reader actually retyping a score would have to do too.
fn retype_score(mut s: State, new: &str) -> State {
    (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::End)));
    for _ in 0..32 {
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Backspace)));
    }
    for c in new.chars() {
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c))));
    }
    s
}

#[test]
fn golden_zset_form_edit_score_shows_the_raw_score_by_member() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "beta"
    open.begin_edit(EditBuffer::zset_score(b"beta", 2.0));
    assert_golden("zset_form_edit_score", &draw(&state, 130, 22));
}

#[test]
fn golden_zset_add_form_member_part_shows_the_placeholder_on_score() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_zset_member());
    let editor = open.typing_mut().unwrap();
    for c in "delta".chars() {
        editor.name_push(c);
    }
    assert_golden("zset_form_add_member", &draw(&state, 130, 22));
}

#[test]
fn golden_zset_add_form_shown_duplicate_blocks_staging() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    let open = state.open.as_mut().unwrap();
    open.begin_edit(EditBuffer::new_zset_member());
    let editor = open.typing_mut().unwrap();
    // "alpha" is one of `zset_value()`'s members — a shown duplicate.
    for c in "alpha".chars() {
        editor.name_push(c);
    }
    assert_golden("zset_form_add_duplicate", &draw(&state, 130, 22));
}

#[test]
fn golden_zset_add_form_score_part_shows_member_above_the_active_editor() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let state = "delta".chars().fold(state, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("12.5");
    assert_golden("zset_form_add_score", &draw(&state, 130, 22));
}

/// D4's live invalid-score indicator: `Ctrl-S`/`Enter` are blocked while the
/// score part does not parse, and the hint bar says so — the same frame
/// comparison the List add form's head/tail toggle test uses to prove its
/// wording actually changed on screen.
#[test]
fn golden_zset_add_form_invalid_score_shows_the_live_indicator() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let state = "delta".chars().fold(state, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    let mut state = state;
    let editor = state.open.as_mut().unwrap().typing_mut().unwrap();
    editor.insert_str("not-a-number");
    // The hint bar carries the live indicator (phase 3's D4 note), and the
    // hint bar only draws at 24 rows or taller (`layout::layout`'s
    // `area.height >= 24`) — asserted directly here, the same way the
    // sibling Hash/List/Set hint-bar tests assert `hint_bar(&state)` rather
    // than searching a shorter frame that would never show it.
    assert!(
        hint_bar(&state, state.cols).contains("invalid score"),
        "the live indicator must be on offer: {}",
        hint_bar(&state, state.cols)
    );
    assert_golden("zset_form_add_invalid_score", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_set_zset_score_shows_the_effective_command_and_its_guard() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    // `e` is focus-gated (ADR-0015 D4, mirrored for ZSets); `opened()` leaves
    // focus on the keys pane.
    state.focus = Pane::Value;
    let open = state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "beta", score 2.0
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    let state = retype_score(state, "10");
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::SetZSetScore { .. })
    ));
    assert_golden("confirm_set_zset_score", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_add_zset_member_shows_the_guard_and_only_a_plus_side() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let state = "delta".chars().fold(state, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    let state = "12.5".chars().fold(state, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::AddZSetMember { .. })
    ));
    assert_golden("confirm_add_zset_member", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_zset_member_shows_the_effective_command() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    // `d` is focus-dependent (D8): without this, `opened()`'s default focus
    // (the keys pane) makes `d` stage `DeleteKey`, not `DeleteZSetMember`.
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteZSetMember {
            last_member: false,
            ..
        })
    ));
    assert_golden("confirm_delete_zset_member", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_delete_zset_member_warns_when_it_is_the_last_member() {
    let mut state = opened(
        "user:8812:cart",
        Value::ZSet(ScoredValue {
            entries: vec![("only".into(), 1.0)],
            total: 1,
        }),
        600,
    );
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteZSetMember {
            last_member: true,
            ..
        })
    ));
    assert_golden(
        "confirm_delete_zset_member_last_member",
        &draw(&state, 130, 22),
    );
}

#[test]
fn golden_hint_bar_names_all_three_zset_actions_with_a_cursor_active() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let hint = hint_bar(&state, state.cols);
    assert!(hint.contains("e score"), "{hint}");
    assert!(
        !hint.contains("e edit"),
        "D1: the ZSet row says score, never edit: {hint}"
    );
    assert!(hint.contains("a add member"), "{hint}");
    assert!(hint.contains("d remove member"), "{hint}");
}

/// PLAN M2 row 9's "Proves" clause, pinned directly: a score edit's dialog
/// must visibly differ from add's and remove's, because it diffs a
/// different thing. A score edit shows the member unchanged with
/// `old → new` on the score alone; add and remove both diff *membership* —
/// the whole `member + score` pair appears on a `+`/`-` side. Three
/// different frames, not three renderings of the same shape.
#[test]
fn a_zset_score_edit_dialog_shows_a_score_diff_distinct_from_a_membership_diff() {
    let mut score_state = opened("user:8812:cart", zset_value(), 720);
    score_state.focus = Pane::Value;
    let open = score_state.open.as_mut().unwrap();
    open.cursor_active = true;
    open.cursor = 1; // "beta", score 2.0
    let (s, _) = update(score_state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    let s = retype_score(s, "10");
    let (score_state, _) = update(s, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    let score_frame = draw(&score_state, 130, 22);
    assert!(score_frame.contains("member beta"), "{score_frame}");
    assert!(score_frame.contains("score 2 → 10"), "{score_frame}");
    assert!(
        !score_frame.contains("+ delta") && !score_frame.contains("- alpha"),
        "a score edit must not draw a membership +/- diff: {score_frame}"
    );

    let mut add_state = opened("user:8812:cart", zset_value(), 720);
    add_state.focus = Pane::Value;
    let (s, _) = update(add_state, Msg::Key(KeyPress::plain(KeyCode::Char('a'))));
    let s = "delta".chars().fold(s, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    let s = "12.5".chars().fold(s, |s, c| {
        update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0
    });
    let (add_state, _) = update(s, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    let add_frame = draw(&add_state, 130, 22);
    assert!(add_frame.contains("+ delta 12.5"), "{add_frame}");
    assert!(
        !add_frame.contains("→"),
        "an add is a membership diff, not a score diff: {add_frame}"
    );

    let mut delete_state = opened("user:8812:cart", zset_value(), 720);
    delete_state.focus = Pane::Value;
    delete_state.open.as_mut().unwrap().cursor_active = true;
    let (delete_state, _) = update(delete_state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    let delete_frame = draw(&delete_state, 130, 22);
    assert!(delete_frame.contains("- alpha"), "{delete_frame}");
    assert!(
        !delete_frame.contains("→"),
        "a remove is a membership diff, not a score diff: {delete_frame}"
    );

    assert_ne!(score_frame, add_frame, "score edit and add must differ");
    assert_ne!(
        score_frame, delete_frame,
        "score edit and remove must differ"
    );
    assert_ne!(add_frame, delete_frame, "add and remove must differ");
}

// ── UI task: type colour dots and the selection bar (style-verified) ────────

use redis_pane_core::theme::Token;

#[test]
fn golden_browser_style_selection_and_type_colours() {
    let state = many_keys();
    let frame = render::frame(
        &state,
        &Theme::new(ColorDepth::TrueColor),
        &CLOCK,
        Rect::new(0, 0, 90, 12),
    );
    assert_golden("browser_style_truecolor", &render::to_golden(&frame));
}

/// Find the y coordinate of the rendered row containing `needle`, by reading
/// the text a real frame produces rather than guessing chrome-row arithmetic.
fn row_of(frame: &ratatui::buffer::Buffer, needle: &str) -> u16 {
    render::to_text(frame)
        .lines()
        .position(|l| l.contains(needle))
        .expect("row not found") as u16
}

#[test]
fn the_selected_row_carries_a_background_all_the_way_across_not_just_on_the_name() {
    // The tricky part of this feature: put() resets style before applying its
    // own, so a background painted once and then written over by later cells
    // would leave holes rather than one continuous bar.
    let state = many_keys(); // selection defaults to row 0: user:8812:cart
    let theme = Theme::new(ColorDepth::TrueColor);
    let frame = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 130, 12));
    let y = row_of(&frame, "user:8812:cart");

    let selected_bg = theme.style(Token::Selected).bg;
    assert!(selected_bg.is_some());

    // The bar spans exactly the keys pane, not the full 130-column frame —
    // there is a value pane to the right of it, correctly unpainted.
    let keys_pane_width = redis_pane_core::render::layout::layout(
        Rect::new(0, 0, 130, 12),
        redis_pane_core::render::layout::Pane::Keys,
        0,
    )
    .keys
    .width;

    let mut gaps = Vec::new();
    for x in 1..keys_pane_width {
        match frame.cell((x, y)) {
            Some(cell) if cell.style().bg == selected_bg => {}
            other => gaps.push((x, other.map(|c| c.style().bg))),
        }
    }
    assert!(gaps.is_empty(), "the selection bar has gaps: {gaps:?}");
}

#[test]
fn an_unselected_row_carries_no_background_at_all() {
    // The selection bar must not bleed into neighbouring rows.
    let state = many_keys();
    let theme = Theme::new(ColorDepth::TrueColor);
    let frame = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 130, 12));
    let y = row_of(&frame, "user:8812:profile"); // row 1, not selected

    let keys_pane_width = redis_pane_core::render::layout::layout(
        Rect::new(0, 0, 130, 12),
        redis_pane_core::render::layout::Pane::Keys,
        0,
    )
    .keys
    .width;
    for x in 1..keys_pane_width {
        let bg = frame.cell((x, y)).map(|c| c.style().bg);
        assert!(
            matches!(bg, Some(None) | Some(Some(ratatui::style::Color::Reset))),
            "row 1 should carry no background, found {bg:?} at column {x}"
        );
    }
}

#[test]
fn distinct_types_render_with_distinct_dot_colours_in_a_real_frame() {
    // many_keys(): cart(zset) profile(json) session(hash) session(hash)
    // cart(zset) hot(list) — enough variety to prove the dots are not all one
    // colour, without hard-coding the exact hue table here. Row 0 (cart) is
    // selected, so its dot is overridden to the selection colour; the rest
    // show their real per-type hue.
    let state = many_keys();
    let theme = Theme::new(ColorDepth::TrueColor);
    let frame = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 130, 12));

    let rows = [
        "user:8812:profile",
        "user:8812:session",
        "cart:91af3c9d2e",
        "feed:global:hot",
    ];
    let dot_colors: Vec<_> = rows
        .iter()
        .map(|needle| {
            let y = row_of(&frame, needle);
            frame.cell((1, y)).map(|c| c.style().fg)
        })
        .collect();
    let unique: std::collections::HashSet<_> = dot_colors.iter().collect();
    assert!(
        unique.len() > 1,
        "every row's dot rendered the same colour: {dot_colors:?}"
    );
}

// ── severity-3 #8: stack navigation below 70 columns ────────────────────────

use redis_pane_core::render::layout::Pane;

#[test]
fn golden_single_pane_value_view_60_cols() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    assert_golden("single_pane_value_60", &draw(&state, 60, 24));
}

#[test]
fn the_default_single_pane_state_still_renders_the_key_list_unchanged() {
    // Regression guard: adding Value mode must not disturb the existing,
    // already-shipped Keys mode at the same width.
    let state = many_keys();
    assert_eq!(state.focus, Pane::Keys);
    let frame = draw(&state, 60, 24);
    assert!(
        frame.contains("KEY"),
        "the flat/tree list header must still be there"
    );
}

#[test]
fn standalone_value_view_has_no_stray_border_character_at_the_left_edge() {
    // The two-pane layouts draw a "│" one column left of the value pane; at
    // full width there is no adjacent pane to separate from, and that
    // character must not appear over the content instead.
    let mut state = opened("k", hash_value(), 600);
    state.focus = Pane::Value;
    let frame = draw(&state, 60, 24);
    for line in frame.lines() {
        assert!(
            !line.starts_with('│'),
            "stray separator at the left edge: {line:?}"
        );
    }
}

#[test]
fn the_breadcrumb_names_the_key_and_the_effective_back_binding() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    let frame = draw(&state, 60, 24);
    assert!(frame.contains("back"), "{frame}");
    assert!(frame.contains("user:8812:session"), "{frame}");

    // R7.5: the breadcrumb must follow a rebinding, not a hard-coded "Esc".
    let mut keymap = redis_pane_core::keymap::Keymap::default();
    keymap.bind(
        redis_pane_core::keymap::Action::Cancel,
        KeyPress::plain(KeyCode::Char('h')),
    );
    state.keymap = keymap;
    let rebound = draw(&state, 60, 24);
    assert!(rebound.contains("h back"), "{rebound}");
}

#[test]
fn a_key_opened_narrow_shows_its_real_value_not_a_placeholder() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    let frame = draw(&state, 60, 24);
    assert!(frame.contains("device"), "{frame}");
    assert!(frame.contains("ios/17.2"), "{frame}");
}

// ── severity-3 #7: the scan cap gets a persistent banner, not a status line ─

fn capped_state() -> State {
    let mut keys = LoadedSet::with_cap(3);
    for name in ["a:1", "a:2", "a:3", "a:4"] {
        keys.push(name.as_bytes());
    }
    assert!(
        keys.is_capped(),
        "the fixture must actually be capped, or this proves nothing"
    );
    let mut state = State {
        keys,
        scan: redis_pane_core::state::ScanState::Capped { at: 3 },
        link: up(Tk::Armed),
        ..base()
    };
    state.rebuild_list();
    state
}

#[test]
fn golden_capped_keyspace() {
    assert_golden("browser_capped", &draw(&capped_state(), 130, 14));
}

fn interrupted_state() -> State {
    let mut state = many_keys();
    state.scan = redis_pane_core::state::ScanState::Interrupted {
        scanned: 6,
        reason: redis_pane_core::state::InterruptReason::TopologyChanged,
    };
    state
}

#[test]
fn golden_scan_interrupted() {
    let frame = draw(&interrupted_state(), 130, 14);
    assert!(
        frame.contains("the cluster changed during the scan — r to rescan"),
        "{frame}"
    );
    assert!(!frame.contains("scanning"), "{frame}");
    assert_golden("browser_interrupted", &frame);
}

#[test]
fn golden_scan_interrupted_narrow() {
    assert_golden(
        "browser_interrupted_60",
        &draw(&interrupted_state(), 60, 14),
    );
}

#[test]
fn the_banner_names_the_cap_and_what_to_do_about_it() {
    let frame = draw(&capped_state(), 130, 14);
    assert!(frame.contains('⚠'), "{frame}");
    assert!(frame.contains("3"), "{frame}");
    assert!(frame.contains("narrow the filter"), "{frame}");
}

#[test]
fn an_uncapped_keyspace_shows_no_banner_and_spends_no_row_on_it() {
    let state = many_keys();
    assert!(!state.keys.is_capped());
    let capped_frame = draw(&capped_state(), 130, 14);
    let plain_frame = draw(&state, 130, 14);
    assert!(!plain_frame.contains('⚠'), "{plain_frame}");
    // The uncapped list's column header must sit one row higher than the
    // capped one's — proof the banner row is genuinely not reserved when it
    // is not needed, not just left blank.
    let header_row = |f: &str| {
        f.lines()
            .position(|l| l.contains("KEY") && l.contains("TYPE"))
    };
    assert!(
        header_row(&plain_frame) < header_row(&capped_frame),
        "plain={:?} capped={:?}",
        header_row(&plain_frame),
        header_row(&capped_frame)
    );
}

#[test]
fn the_banner_survives_filtering_because_the_underlying_set_is_still_incomplete() {
    // Filtering narrows what is shown, not what was actually scanned — the
    // set stays capped, and hiding that fact behind a short match list would
    // be worse: a filtered "no results" would look identical to "we never
    // got that far."
    let mut state = capped_state();
    state.list.filter = "a:1".into();
    state.rebuild_list();
    assert!(state.keys.is_capped());
    let frame = draw(&state, 130, 14);
    assert!(frame.contains('⚠'), "{frame}");
    assert!(
        frame.contains('/'),
        "the filter line must still be there too: {frame}"
    );
}

#[test]
fn the_banner_is_not_displaced_by_a_copy_confirmation() {
    // The entire point: today's status-bar line can be overwritten by
    // anything else that wants that row for a few seconds. This one cannot.
    let mut state = capped_state();
    let (next, _) = update(
        state.clone(),
        Msg::Copied {
            label: "key".into(),
            at_ms: 73_000,
        },
    );
    state = next;
    let frame = draw(&state, 130, 14);
    assert!(
        frame.contains('⚠'),
        "the cap banner must survive a transient notice: {frame}"
    );
    assert!(
        frame.contains("copied key"),
        "the notice itself should still show too: {frame}"
    );
}

#[test]
fn the_banner_coexists_with_the_filter_line_in_the_documented_order() {
    let mut state = capped_state();
    state.list.filter = "a:".into();
    state.rebuild_list();
    let frame = draw(&state, 130, 14);
    let lines: Vec<&str> = frame.lines().collect();
    let banner_row = lines.iter().position(|l| l.contains('⚠')).unwrap();
    let filter_row = lines.iter().position(|l| l.starts_with(" /")).unwrap();
    assert!(
        banner_row < filter_row,
        "the cap banner should sit above the filter line"
    );
}

// ── The Open key vs the Selected key (CONTEXT.md; UI task severity 1) ───────
//
// The reported defect: the cursor on one key, the value pane showing another,
// with nothing on screen relating the two. The value was never wrong — it was a
// live tracked read of a different key — so these fixtures are about identity,
// not freshness.

/// The Attached case is the quiet one: no wash, a solid divider, no chip. It is
/// pinned here so that "nothing happens when the panes agree" is a tested
/// property rather than an assumption.
#[test]
fn golden_viewer_attached_says_nothing_extra() {
    let state = opened("user:8812:session", hash_value(), 2_537);
    let frame = draw(&state, 130, 22);
    assert!(!frame.contains('┊'), "no dashed divider while attached");
    assert!(!frame.contains('⊘'), "no chip while attached");
    assert!(frame.contains('├'), "but the row is still tied to the pane");
}

/// The cursor moves off the Open key. This is the frame the bug report was
/// about, and it is the one that has to be unmistakable.
#[test]
fn golden_viewer_detached() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.view.selected = 0; // `user:8812:cart`, two rows above the Open key
    assert_golden("viewer_detached", &draw(&state, 130, 22));
}

/// Filtered out: the Open key has no row, so the keys pane has nothing to mark
/// and the Viewer has to carry the whole signal on its own.
#[test]
fn golden_viewer_detached_off_list() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.list.filter = "cart".into();
    state.rebuild_list();
    assert_golden("viewer_detached_off_list", &draw(&state, 130, 22));
}

/// Scrolled past: the Open key is real and has a row, but not one on screen.
/// The divider points the way rather than leaving it to be hunted for.
#[test]
fn golden_viewer_detached_scrolled_out_of_view() {
    let mut state = opened("user:8812:cart", hash_value(), 2_537);
    // Enough rows that the window cannot hold them all, which is the only way
    // the Open key can have a row and still not be on screen.
    for i in 0..40 {
        state.keys.push(format!("filler:{i:03}").as_bytes());
    }
    state.rebuild_list();
    state.view.selected = state.row_count() - 1;
    let frame = draw(&state, 130, 22);
    assert!(
        frame.contains('▲'),
        "the Open key is above the window:\n{frame}"
    );
    assert_golden("viewer_detached_scrolled_out", &frame);
}

/// Below 70 columns there is no second pane, so the divider, the tie glyph and
/// the row underline are all gone by construction — and the wash is nothing in
/// monochrome. The chip is the only signal left, and it used to be drawn only
/// in the two-pane branch: every fixture written for this feature was 130 or 80
/// columns wide, so nothing caught it.
#[test]
fn golden_viewer_detached_single_pane() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.cols = 60;
    state.focus = redis_pane_core::render::layout::Pane::Value;
    state.view.selected = 0;

    let frame = draw(&state, 60, 22);
    assert!(
        frame.contains('⊘'),
        "the one signal a single pane can carry:\n{frame}"
    );
    assert!(
        frame.contains("user:8812:session"),
        "and the breadcrumb still names the key:\n{frame}"
    );
    assert_golden("viewer_detached_single", &frame);
}

// ── a key confirmed gone before it was ever loaded (severity 1) ─────────────
//
// Reported from use: arrowing onto an already-deleted key while a different
// one was open left the Viewer showing the *previous* key's value, badged
// `✕ deleted` — which was actually a separate bug (the badge was attributed
// to the wrong key; see `update.rs`'s `opening_a_gone_key_never_...` tests)
// layered under a real design question: even correctly attributed, a value
// that was never read has no type, size, or TTL to show, and no question of
// attachment to answer. These fixtures pin the minimal render: the name, and
// that it is gone — nothing else.

/// A key requested and confirmed gone, on the row the cursor is on.
fn gone_key(name: &str) -> State {
    let mut state = many_keys();
    let index = (0..state.keys.len())
        .find(|&i| state.keys.name_str(i).as_deref() == Some(name))
        .unwrap_or_else(|| {
            state.keys.push(name.as_bytes());
            state.keys.len() - 1
        });
    state.open = Some(OpenKey::gone(Some(index), name.into(), 60_000));
    state.rebuild_list();
    state.view.selected = state.open.as_ref().and_then(|open| open.row).unwrap_or(0);
    state
}

#[test]
fn golden_viewer_gone_before_load() {
    let frame = draw(&gone_key("bighash"), 130, 22);
    assert!(frame.contains("bighash"), "{frame}");
    assert!(frame.contains("✕ gone"), "{frame}");
    assert!(
        !frame.contains("ttl"),
        "no ttl line for a value never read:\n{frame}"
    );
    assert!(!frame.contains('⊘'), "no attachment chip either:\n{frame}");
    assert_golden("viewer_gone_before_load", &frame);
}

/// The same state, but the cursor has since moved off the gone key's row.
/// Per the explicit design choice, this state skips the attachment
/// disclosure entirely — no wash, no dashed divider, no chip — even though
/// the key genuinely is not the one selected. The two fixtures must be
/// byte-identical in the value pane itself; only the keys pane's cursor row
/// differs.
#[test]
fn golden_viewer_gone_before_load_while_detached() {
    let mut state = gone_key("bighash");
    state.view.selected = 0;
    let frame = draw(&state, 130, 22);
    assert!(frame.contains("bighash"), "{frame}");
    assert!(frame.contains("✕ gone"), "{frame}");
    assert!(
        !frame.contains('⊘'),
        "detachment is not disclosed for a key with nothing to disclose:\n{frame}"
    );
    assert_golden("viewer_gone_before_load_detached", &frame);
}

#[test]
fn golden_viewer_gone_before_load_single_pane() {
    let mut state = gone_key("bighash");
    state.cols = 60;
    state.focus = redis_pane_core::render::layout::Pane::Value;
    let frame = draw(&state, 60, 22);
    assert!(frame.contains("bighash"), "{frame}");
    assert!(frame.contains("✕ gone"), "{frame}");
    assert_golden("viewer_gone_before_load_single", &frame);
}

// ── loading indicator: a read in flight (PLAN, fetch-latency UX) ────────────

#[test]
fn golden_viewer_opening() {
    // Nothing has been read yet — the First Open case. The body must show a
    // placeholder, never a blank pane or a previous key's value.
    use redis_pane_core::command::ReadToken;
    use redis_pane_core::state::PendingRead;

    let mut state = many_keys();
    state.open_pending = Some(PendingRead {
        name: "user:8812:session".into(),
        token: ReadToken::default(),
        index: Some(2),
        // Well past `APPEAR_DELAY_MS` relative to `CLOCK` (74_000): this read
        // is genuinely slow, so the placeholder must show.
        issued_at_ms: Some(73_000),
        activate_cursor: false,
        own_write: false,
    });
    let frame = draw(&state, 130, 22);
    assert!(frame.contains("user:8812:session"), "{frame}");
    assert!(frame.contains("⟳ fetching…"), "{frame}");
    assert!(
        !frame.contains("ttl"),
        "no ttl line for a value that has not arrived:\n{frame}"
    );
    assert_golden("viewer_opening", &frame);
}

/// The regression this fix is actually about: on a fast local Redis the read
/// lands well inside `APPEAR_DELAY_MS`, and the indicator used to flash on and
/// off within a frame or two — a glitch, not feedback. A read that has not
/// been outstanding long enough must render *identically* to no pending read
/// at all, whether it is merely too recent or has not been stamped yet
/// (`issued_at_ms: None`, before the shell's `Msg::ReadIssued` arrives).
#[test]
fn golden_viewer_opening_a_fast_read_shows_nothing_at_all() {
    use redis_pane_core::command::ReadToken;
    use redis_pane_core::state::PendingRead;

    let baseline = draw(&many_keys(), 130, 22);

    let mut too_recent = many_keys();
    too_recent.open_pending = Some(PendingRead {
        name: "user:8812:session".into(),
        token: ReadToken::default(),
        index: Some(2),
        // 100ms elapsed against `CLOCK` (74_000) — under the 200ms gate.
        issued_at_ms: Some(73_900),
        activate_cursor: false,
        own_write: false,
    });
    let frame = draw(&too_recent, 130, 22);
    assert_eq!(
        frame, baseline,
        "a read 100ms old must render exactly like no pending read:\n{frame}"
    );

    let mut unstamped = many_keys();
    unstamped.open_pending = Some(PendingRead {
        name: "user:8812:session".into(),
        token: ReadToken::default(),
        index: Some(2),
        issued_at_ms: None,
        activate_cursor: false,
        own_write: false,
    });
    let frame = draw(&unstamped, 130, 22);
    assert_eq!(
        frame, baseline,
        "an unstamped pending read must render exactly like no pending read:\n{frame}"
    );
}

#[test]
fn golden_viewer_opening_a_different_key_than_the_one_already_shown() {
    // A key is open and showing its value; the reader opens a different key
    // before the reply lands. The old value must not linger on screen next
    // to the new key's name — the placeholder replaces it wholesale.
    use redis_pane_core::command::ReadToken;
    use redis_pane_core::state::PendingRead;

    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.open_pending = Some(PendingRead {
        name: "user:8812:cart".into(),
        token: ReadToken::default(),
        index: Some(0),
        issued_at_ms: Some(73_000),
        activate_cursor: false,
        own_write: false,
    });
    let frame = draw(&state, 130, 22);
    assert!(frame.contains("user:8812:cart"), "{frame}");
    assert!(
        !frame.contains("device") && !frame.contains("ios/17.2"),
        "the previous key's value must not linger:\n{frame}"
    );
    assert!(frame.contains("⟳ fetching…"), "{frame}");
    assert_golden("viewer_opening_different_key", &frame);
}

#[test]
fn golden_viewer_refetching() {
    // The key already open is being re-read (invalidation re-arm, manual
    // Refetch, …). The value already on screen must stay exactly as it is —
    // only the status text says a read is outstanding.
    use redis_pane_core::command::ReadToken;
    use redis_pane_core::state::PendingRead;

    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.open_pending = Some(PendingRead {
        name: "user:8812:session".into(),
        token: ReadToken::default(),
        index: state.open.as_ref().unwrap().index,
        issued_at_ms: Some(73_000),
        activate_cursor: false,
        own_write: false,
    });
    let frame = draw(&state, 130, 22);
    assert!(
        frame.contains("device"),
        "the value stays on screen:\n{frame}"
    );
    assert!(
        frame.contains("id") && frame.contains("8812") && frame.contains("locale"),
        "every field is still there, unchanged:\n{frame}"
    );
    assert!(frame.contains("⟳ fetching…"), "{frame}");
    assert_golden("viewer_refetching", &frame);
}

/// The sharper version of the same regression: an invalidation re-arm fires
/// on every write to the key, so an ungated Refetch indicator would flicker
/// on essentially every keystroke against a live server, not just on Open.
#[test]
fn golden_viewer_refetching_a_fast_reply_shows_nothing_at_all() {
    use redis_pane_core::command::ReadToken;
    use redis_pane_core::state::PendingRead;

    let baseline = draw(&opened("user:8812:session", hash_value(), 2_537), 130, 22);

    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.open_pending = Some(PendingRead {
        name: "user:8812:session".into(),
        token: ReadToken::default(),
        index: state.open.as_ref().unwrap().index,
        issued_at_ms: Some(73_900),
        activate_cursor: false,
        own_write: false,
    });
    let frame = draw(&state, 130, 22);
    assert_eq!(
        frame, baseline,
        "a refetch 100ms old must render exactly like no pending read:\n{frame}"
    );
}

// The preserved case — a key that *was* loaded, then deleted while open,
// keeps its full value and header exactly as before, because ADR-0006's
// "what was in it" question still has an answer for it — is already pinned
// by `golden_viewer_deleted` above, unmodified by this change: it passed
// byte-for-byte with everything else in this file once `OpenKey.value`
// became optional, which is the regression guarantee this comment is here to
// point at rather than duplicate.

/// The chip gives way before the key name does, following the title bar's rule:
/// the name is the pane's identity, the chip is a qualifier on it.
#[test]
fn the_chip_shortens_and_then_goes_rather_than_crowding_the_key_name() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.view.selected = 0;
    let wide = draw(&state, 130, 22);
    assert!(wide.contains("⊘ not the selected key"), "{wide}");

    let narrow = draw(&state, 80, 22);
    assert!(
        narrow.contains("user:8812:session"),
        "the key name always survives:\n{narrow}"
    );
    assert!(
        narrow.contains('⊘'),
        "and the chip is still stated:\n{narrow}"
    );
    assert!(
        !narrow.contains("⊘ not the selected key"),
        "but not at full length in a pane this narrow:\n{narrow}"
    );
}

/// The wash is hue and nothing else, so monochrome must lose it — and must
/// still say the same thing. This is the test that stops the loud treatment
/// from becoming the *only* treatment.
#[test]
fn detachment_survives_the_loss_of_colour() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.view.selected = 0;
    let area = Rect::new(0, 0, 130, 22);

    // `put` writes `Style::reset()`, which leaves `bg` as `Some(Color::Reset)`.
    // That is the *absence* of a background, so testing `is_some()` would pass
    // on every cell in the frame and prove nothing.
    let washed_at = |buf: &ratatui::buffer::Buffer, x: u16, y: u16| {
        !matches!(
            buf.cell((x, y)).map(|c| c.style().bg),
            None | Some(None) | Some(Some(ratatui::style::Color::Reset))
        )
    };

    let color = render::frame(&state, &Theme::new(ColorDepth::TrueColor), &CLOCK, area);
    // The pane's own rows are exactly the ones the dashed divider runs down, so
    // the two signals are checked against each other rather than against a
    // hand-counted range of chrome rows.
    let pane_rows: Vec<u16> = render::to_text(&color)
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains('┊'))
        .map(|(y, _)| y as u16)
        .collect();
    assert!(pane_rows.len() > 10, "sanity: {pane_rows:?}");
    assert!(
        pane_rows.iter().all(|y| washed_at(&color, 100, *y)),
        "in colour the whole value pane is washed, empty rows included"
    );
    assert!(
        !washed_at(&color, 20, 4),
        "and the keys pane is not — the wash is a statement about one pane"
    );

    let mono = render::frame(&state, &Theme::new(ColorDepth::Monochrome), &CLOCK, area);
    assert!(
        (0..area.height).all(|y| !washed_at(&mono, 100, y)),
        "in monochrome there is no wash at all"
    );
    let text = render::to_text(&mono);
    assert!(text.contains('┊'), "the dashed divider carries it instead");
    assert!(
        text.contains("⊘ not the selected key"),
        "and so does the chip"
    );
}

/// Two marks in one list only work if one is obviously the junior partner. The
/// cursor keeps reverse video; the Open key's row gets underline, which is the
/// one modifier still free once the selection has taken the other.
#[test]
fn the_open_row_is_underlined_and_the_cursor_row_is_not_merely_that() {
    use ratatui::style::Modifier;

    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.view.selected = 0;
    let area = Rect::new(0, 0, 130, 22);
    let mono = render::frame(&state, &Theme::new(ColorDepth::Monochrome), &CLOCK, area);

    // The bare name also appears in the value pane's header, which is above the
    // list; the dot is what makes this a key *row*.
    let open_y = row_of(&mono, "● user:8812:session");
    let cursor_y = row_of(&mono, "● user:8812:cart");
    assert_ne!(open_y, cursor_y, "this fixture is only meaningful detached");

    let style_at = |x: u16, y: u16| {
        mono.cell((x, y))
            .map(|c| c.style())
            .expect("cell in bounds")
    };
    // Column 3 is inside the key name, past the leading space and the dot.
    assert!(
        style_at(3, open_y)
            .add_modifier
            .contains(Modifier::UNDERLINED),
        "the Open key's name is underlined"
    );
    assert!(
        style_at(3, cursor_y)
            .add_modifier
            .contains(Modifier::REVERSED),
        "the cursor's row keeps the full-bar highlight"
    );
    assert!(
        !style_at(3, cursor_y)
            .add_modifier
            .contains(Modifier::UNDERLINED),
        "and the two marks are not the same mark"
    );
}

/// Drawing is where the editor learns the pane's width. If the frame drew a
/// copy, `Down` in a value with no newlines would have no row to go to.
#[test]
fn down_moves_by_wrapped_row_in_a_value_with_no_newlines_once_drawn() {
    let token = "x".repeat(400);
    let mut state = opened(
        "user:8812:token",
        Value::Str(StringValue::new(&token, 65)),
        600,
    );
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    draw(&state, 130, 22);
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Down)));
    let cursor = state
        .open
        .as_ref()
        .unwrap()
        .editor()
        .unwrap()
        .widget()
        .cursor();
    let (line, col) = (cursor.0, cursor.1);
    assert_eq!(line, 0, "still the one logical line");
    assert!(
        col > 0,
        "moved one wrapped row down, not nowhere: col {col}"
    );
}

/// The crate's own cursor is reverse video, which on light text reads as
/// barely anything. It is repainted with the Viewer's cursor token instead.
#[test]
fn the_editor_cursor_is_drawn_with_the_viewers_cursor_colour() {
    use ratatui::style::Modifier;
    use redis_pane_core::theme::Token;
    let token = "x".repeat(400);
    let mut state = opened(
        "user:8812:token",
        Value::Str(StringValue::new(&token, 65)),
        600,
    );
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    let theme = Theme::new(ColorDepth::TrueColor);
    let buf = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 130, 22));
    let selected = theme.style(Token::Selected);
    let cursor_cells = (1..22)
        .flat_map(|y| (0..130).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let cell = &buf[(x, y)];
            cell.symbol() == "x" && Some(cell.bg) == selected.bg && Some(cell.fg) == selected.fg
        })
        .count();
    assert_eq!(
        cursor_cells, 1,
        "exactly the cursor, on a character of the value"
    );
    assert!(
        !buf.content
            .iter()
            .any(|c| c.modifier.contains(Modifier::REVERSED)),
        "no reverse-video cursor left over"
    );
}

// ── PLAN M2 task 10 — TTL editing (ADR-0019) ────────────────────────────────

/// Open the TTL field through the real keypress path — `t`'s focus split
/// (D2) — so these frames also pin the dispatch, not just the drawing.
fn open_ttl_field(mut state: State) -> State {
    state.focus = Pane::Value;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('t'))));
    assert!(
        state.open.as_ref().unwrap().is_editing(),
        "t in the value pane must open the TTL field"
    );
    state
}

/// Clear the seeded duration and type a replacement.
///
/// Unlike `retype_score` above, no `End` is needed first: the TTL capture is
/// the hand-painted single-line one (D11), which has no cursor to be in the
/// wrong place, so `Backspace` removes the last character as a reader would
/// expect. That difference is the point of D11 and is worth seeing here.
fn retype_ttl(mut s: State, new: &str) -> State {
    for _ in 0..24 {
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Backspace)));
    }
    for c in new.chars() {
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char(c))));
    }
    s
}

#[test]
fn golden_ttl_form_seeded_from_the_keys_current_ttl() {
    let state = open_ttl_field(opened("session:9f3a", hash_value(), 2_520));
    assert_golden("ttl_form_seeded", &draw(&state, 130, 22));
}

/// A key with no expiry seeds empty, and the line under the field teaches the
/// grammar rather than refusing a field nobody has typed into yet (D11).
#[test]
fn golden_ttl_form_empty_shows_the_grammar_as_a_placeholder() {
    let state = open_ttl_field(opened("session:9f3a", hash_value(), -1));
    assert_golden("ttl_form_empty_placeholder", &draw(&state, 130, 22));
}

#[test]
fn golden_ttl_form_resolves_a_set_while_typing() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
        "5m",
    );
    assert_golden("ttl_form_set", &draw(&state, 130, 22));
}

#[test]
fn golden_ttl_form_resolves_an_extend_while_typing() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
        "+30m",
    );
    assert_golden("ttl_form_extend", &draw(&state, 130, 22));
}

#[test]
fn golden_ttl_form_resolves_a_shorten_while_typing() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
        "-10m",
    );
    assert_golden("ttl_form_shorten", &draw(&state, 130, 22));
}

/// Clearing the field entirely is how persist is asked for on a key that
/// currently has an expiry.
#[test]
fn golden_ttl_form_resolves_a_persist_when_cleared() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
        "",
    );
    assert_golden("ttl_form_persist", &draw(&state, 130, 22));
}

/// The refusal that matters most: `EXPIRE key 0` deletes the key, so the
/// field names `d` rather than sending it (D4).
#[test]
fn golden_ttl_form_refuses_zero_and_names_the_delete_key() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
        "0",
    );
    assert_golden("ttl_form_zero_deletes", &draw(&state, 130, 22));
}

#[test]
fn golden_ttl_form_refuses_text_it_cannot_read() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
        "1.5h",
    );
    assert_golden("ttl_form_unreadable", &draw(&state, 130, 22));
}

/// A shift needs an expiry to shift. Refused with wording that names the fix,
/// rather than silently doing nothing (D4).
#[test]
fn golden_ttl_form_refuses_a_shift_on_a_key_with_no_expiry() {
    let state = retype_ttl(
        open_ttl_field(opened("session:9f3a", hash_value(), -1)),
        "+30m",
    );
    assert_golden("ttl_form_no_expiry", &draw(&state, 130, 22));
}

fn stage_ttl(state: State, typed: &str) -> State {
    let state = retype_ttl(open_ttl_field(state), typed);
    let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('s'))));
    state
}

#[test]
fn golden_confirm_set_ttl_shows_the_command_its_guard_and_the_diff() {
    let state = stage_ttl(opened("session:9f3a", hash_value(), 2_520), "5m");
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::SetTtl { .. })
    ));
    assert_golden("confirm_set_ttl", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_persist_ttl_shows_the_expiry_being_cleared() {
    let state = stage_ttl(opened("session:9f3a", hash_value(), 2_520), "");
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::PersistTtl { .. })
    ));
    assert_golden("confirm_persist_ttl", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_shift_ttl_shows_both_guard_clauses() {
    let state = stage_ttl(opened("session:9f3a", hash_value(), 2_520), "+30m");
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::ShiftTtl { .. })
    ));
    assert_golden("confirm_shift_ttl", &draw(&state, 130, 22));
}

/// Giving a permanent key an expiry is the risky direction — on `prod` it is
/// how data goes missing at 3am — so it carries a warning, the TTL analogue
/// of "last member — the key will be deleted" (D9).
#[test]
fn golden_confirm_set_ttl_warns_when_the_key_had_no_expiry() {
    let state = stage_ttl(opened("session:9f3a", hash_value(), -1), "5m");
    assert_golden("confirm_set_ttl_had_no_expiry", &draw(&state, 130, 22));
}

/// PLAN row 10 asks the preview to distinguish the three writes. Pinned as a
/// frame inequality rather than left implied by three separate fixtures.
#[test]
fn golden_the_three_ttl_dialogs_are_visibly_different() {
    let base = || opened("session:9f3a", hash_value(), 2_520);
    let set = draw(&stage_ttl(base(), "5m"), 130, 22);
    let shift = draw(&stage_ttl(base(), "+30m"), 130, 22);
    let persist = draw(&stage_ttl(base(), ""), 130, 22);

    assert_ne!(set, shift, "a set and an extend must not read alike");
    assert_ne!(set, persist, "a set and a persist must not read alike");
    assert_ne!(
        shift, persist,
        "an extend and a persist must not read alike"
    );

    assert!(set.contains("EXPIRE session:9f3a 300"), "{set}");
    assert!(shift.contains("+30m"), "{shift}");
    assert!(persist.contains("PERSIST session:9f3a"), "{persist}");
    assert!(
        shift.contains("never expires it immediately"),
        "only the shift carries the second guard clause\n{shift}"
    );
}

/// D13: the TTL field's live indicator is the resolution line under it, not
/// the hint bar — so the bar stays constant, unlike the ZSet score edit's.
#[test]
fn golden_ttl_hint_bar_is_constant_whatever_is_typed() {
    let state = open_ttl_field(opened("session:9f3a", hash_value(), 2_520));
    let resting = render::hint_bar(&state, state.cols);
    // "never persists" is `help::status`'s prefix now (M3), not a row of its
    // own, and `F1 help` is pinned last — the meaning pinned here (the bar
    // does not change with what's typed) is unaffected; only the layout is.
    assert_eq!(resting, "never persists   ⌃S apply   Esc cancel   F1 help");
    for typed in ["5m", "+30m", "0", "1.5h", ""] {
        let s = retype_ttl(
            open_ttl_field(opened("session:9f3a", hash_value(), 2_520)),
            typed,
        );
        assert_eq!(
            render::hint_bar(&s, s.cols),
            resting,
            "the bar must not change for {typed:?} — the resolution line carries that"
        );
    }
}

// ── M3 — contextual help overlay, per context, at 80 and 130 columns ───────
//
// `help_overlay.txt`/`help_overlay_frame.txt` (the old flat every-binding
// list) are replaced by one fixture per named context, at both widths PLAN's
// verification list asks for. Each is the whole frame — title bar, keys
// pane, hint bar underneath, and the overlay on top — so a regression in
// how the overlay composes with the rest of the screen shows up here too,
// the same way `golden_help_overlay_frame` (still kept, unnamed-context)
// already did for the old design.

fn help_frame(mut state: State, cols: u16) -> String {
    state.help = Some(HelpView { pane: state.focus });
    draw(&state, cols, 24)
}

#[test]
fn golden_help_keys_pane_flat() {
    let state = many_keys();
    assert_eq!(state.focus, Pane::Keys);
    assert!(!state.tree_mode, "many_keys() starts flat (State::default)");
    assert_golden("help_keys_flat_80", &help_frame(state.clone(), 80));
    assert_golden("help_keys_flat_130", &help_frame(state, 130));
}

#[test]
fn golden_help_keys_pane_tree() {
    let mut state = many_keys();
    state.tree_mode = true;
    state.rebuild_list();
    assert_golden("help_keys_tree_80", &help_frame(state.clone(), 80));
    assert_golden("help_keys_tree_130", &help_frame(state, 130));
}

#[test]
fn golden_help_value_hash_cursor_on() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    assert_golden("help_value_hash_cursor_80", &help_frame(state.clone(), 80));
    assert_golden("help_value_hash_cursor_130", &help_frame(state, 130));
}

#[test]
fn golden_help_value_set_cursor_on() {
    let mut state = opened("user:8812:session", set_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    assert_golden("help_value_set_cursor_80", &help_frame(state.clone(), 80));
    assert_golden("help_value_set_cursor_130", &help_frame(state, 130));
}

#[test]
fn golden_help_value_zset_cursor_on() {
    let mut state = opened("user:8812:cart", zset_value(), 720);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    assert_golden("help_value_zset_cursor_80", &help_frame(state.clone(), 80));
    assert_golden("help_value_zset_cursor_130", &help_frame(state, 130));
}

#[test]
fn golden_help_value_string() {
    let mut state = opened(
        "user:8812:session",
        Value::Str(StringValue::new("v1", 65)),
        600,
    );
    state.focus = Pane::Value;
    assert_golden("help_value_string_80", &help_frame(state.clone(), 80));
    assert_golden("help_value_string_130", &help_frame(state, 130));
}

#[test]
fn golden_help_editor_hash_field() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('e'))));
    assert!(
        state.open.as_ref().unwrap().typing().is_some(),
        "editor open"
    );
    assert_golden("help_editor_hash_field_80", &help_frame(state.clone(), 80));
    assert_golden("help_editor_hash_field_130", &help_frame(state, 130));
}

#[test]
fn golden_help_confirm() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
    assert!(
        matches!(state.confirm, Some(PendingMutation::DeleteHashField { .. })),
        "a mutation must be staged"
    );
    assert_golden("help_confirm_80", &help_frame(state.clone(), 80));
    assert_golden("help_confirm_130", &help_frame(state, 130));
}

#[test]
fn golden_help_read_only_dimmed() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Value;
    state.open.as_mut().unwrap().cursor_active = true;
    state.read_only = Some(ReadOnlyReason::Environment);
    assert_golden("help_read_only_dimmed_80", &help_frame(state.clone(), 80));
    assert_golden("help_read_only_dimmed_130", &help_frame(state, 130));
}

#[test]
fn golden_help_disconnected() {
    let mut state = many_keys();
    state.link = Link::Reconnecting {
        attempt: 1,
        retry_in_ms: None,
    };
    assert_golden("help_disconnected_80", &help_frame(state.clone(), 80));
    assert_golden("help_disconnected_130", &help_frame(state, 130));
}

// ── M3 — the Slowlog view (`g s`, R6.4, `docs/plans/m3-slowlog.md`) ────────

/// A realistic wall-clock reading, unlike the shared `CLOCK`/`base()`
/// convention's 74 seconds-since-epoch — the Slowlog view's AGE column
/// needs entries meaningfully hours and days apart, which only makes sense
/// against a "now" that is not itself seconds past the epoch.
const SLOWLOG_NOW_MS: u64 = 1_700_000_074_000; // 2023-11-14 22:14:34 UTC
const SLOWLOG_NOW_S: i64 = 1_700_000_074;

fn slowlog_entry(
    id: i64,
    timestamp: i64,
    duration_us: i64,
    command: &str,
    client_addr: &str,
    client_name: &str,
) -> redis_pane_core::state::SlowlogEntry {
    redis_pane_core::state::SlowlogEntry {
        node: String::new(),
        id,
        timestamp,
        duration_us,
        command: command.as_bytes().to_vec(),
        client_addr: client_addr.as_bytes().to_vec(),
        client_name: client_name.as_bytes().to_vec(),
    }
}

fn slowlog_with_entries() -> State {
    let mut state = State {
        screen: redis_pane_core::state::View::Slowlog,
        link: up(Tk::Armed),
        ..base()
    };
    state.slowlog.set_entries(vec![
        slowlog_entry(
            12,
            SLOWLOG_NOW_S - 12, // 12s ago
            185_000,
            "HGETALL user:8812:session",
            "10.0.0.4:51820",
            "worker-3",
        ),
        slowlog_entry(
            11,
            SLOWLOG_NOW_S - 3 * 3_600, // 3h ago
            420,
            "GET lock:checkout:8812",
            "10.0.0.9:33012",
            "",
        ),
        slowlog_entry(
            10,
            SLOWLOG_NOW_S - 2 * 86_400, // 2d ago
            2_400_000,
            "KEYS user:*",
            "10.0.0.4:51820",
            "worker-3",
        ),
    ]);
    state
}

fn draw_at(state: &State, w: u16, h: u16, clock: &dyn Clock) -> String {
    let buf = render::frame(
        state,
        &Theme::new(ColorDepth::Monochrome),
        clock,
        Rect::new(0, 0, w, h),
    );
    render::to_text(&buf)
}

#[test]
fn golden_slowlog_80_cols_with_entries() {
    assert_golden(
        "slowlog_80",
        &draw_at(&slowlog_with_entries(), 80, 24, &FixedClock(SLOWLOG_NOW_MS)),
    );
}

#[test]
fn golden_slowlog_130_cols_with_entries() {
    assert_golden(
        "slowlog_130",
        &draw_at(
            &slowlog_with_entries(),
            130,
            26,
            &FixedClock(SLOWLOG_NOW_MS),
        ),
    );
}

#[test]
fn golden_slowlog_empty() {
    let state = State {
        screen: redis_pane_core::state::View::Slowlog,
        link: up(Tk::Armed),
        ..base()
    };
    assert!(state.slowlog.is_empty());
    assert_golden("slowlog_empty", &draw(&state, 80, 24));
}

#[test]
fn golden_slowlog_error() {
    let mut state = State {
        screen: redis_pane_core::state::View::Slowlog,
        link: up(Tk::Armed),
        ..base()
    };
    state.slowlog.error = Some("ERR unknown command 'SLOWLOG'".to_string());
    assert_golden("slowlog_error", &draw(&state, 80, 24));
}

#[test]
fn golden_pending_chord_hint_bar() {
    let mut state = many_keys();
    state.pending_chord = Some(redis_pane_core::msg::KeyPress::plain(
        redis_pane_core::msg::KeyCode::Char('g'),
    ));
    assert_golden(
        "pending_chord_hint_bar",
        &render::hint_bar(&state, state.cols),
    );
}

#[test]
fn golden_help_in_slowlog_view() {
    // Not `help_frame` (which draws with the shared `CLOCK`, 74 seconds
    // since the epoch): against timestamps this far in the future of that,
    // every AGE cell would clamp to `0s ago`, which proves nothing about
    // the column this task added. `SLOWLOG_NOW_MS` is the realistic clock
    // `slowlog_with_entries` was built against.
    let mut state = slowlog_with_entries();
    state.help = Some(HelpView { pane: state.focus });
    let clock = FixedClock(SLOWLOG_NOW_MS);
    assert_golden("help_slowlog_80", &draw_at(&state, 80, 24, &clock));
    assert_golden("help_slowlog_130", &draw_at(&state, 130, 24, &clock));
}

/// M3 phase B: `d reset slowlog` dims `· preview only` under Read-only Mode
/// exactly like every other mutation-starting key (Decision 4,
/// `docs/plans/m3-slowlog.md`).
#[test]
fn golden_help_in_slowlog_view_read_only() {
    let mut state = slowlog_with_entries();
    state.read_only = Some(ReadOnlyReason::Environment);
    state.help = Some(HelpView { pane: state.focus });
    let clock = FixedClock(SLOWLOG_NOW_MS);
    assert_golden(
        "help_slowlog_read_only_80",
        &draw_at(&state, 80, 24, &clock),
    );
}

/// M3 phase B: the `RESET` confirm dialog (Decision 8) — a one-line preview,
/// no guard, no diff, over the Slowlog screen it was staged from.
#[test]
fn golden_confirm_reset_slowlog() {
    let mut state = slowlog_with_entries();
    state.confirm = Some(PendingMutation::ResetSlowlog);
    assert_golden(
        "confirm_reset_slowlog_80",
        &draw_at(&state, 80, 24, &FixedClock(SLOWLOG_NOW_MS)),
    );
}

#[test]
fn a_duration_over_the_threshold_uses_the_warn_token_not_muted() {
    use redis_pane_core::theme::Token;
    let mut state = slowlog_with_entries();
    // Row 0 defaults to selected, which paints the whole row in `Selected`
    // regardless of duration — move off it so this checks the unselected
    // colour a reader actually scans the list by.
    state.slowlog.selected = 2;
    let theme = Theme::new(ColorDepth::TrueColor);
    let buf = render::frame(
        &state,
        &theme,
        &FixedClock(SLOWLOG_NOW_MS),
        Rect::new(0, 0, 80, 24),
    );
    // Body starts at y=2 (title bar), summary at y=2, column header at y=3,
    // entries from y=4; DURATION starts at x = 1 + AGE_W (10) = 11.
    let warn_cell = buf.cell((11, 4)).unwrap();
    assert_eq!(
        warn_cell.style().fg,
        theme.style(Token::Warn).fg,
        "185ms is over the threshold"
    );
    let muted_cell = buf.cell((11, 5)).unwrap();
    assert_eq!(
        muted_cell.style().fg,
        theme.style(Token::Muted).fg,
        "420µs is under the threshold"
    );
}

// ── Monitor (`g m`, M3 phase B, `docs/plans/m3-monitor.md`) ────────────────

use redis_pane_core::command::FeedKindMsg;
use redis_pane_core::state::FeedStatus;

const MONITOR_NOW_MS: u64 = 1_339_518_083_500;

fn monitor_line(at_ms: u64, raw: &str) -> (u64, String) {
    (at_ms, raw.to_string())
}

/// A live, open feed with three lines already in the tail — the shape most
/// Monitor goldens start from.
fn monitor_with_lines() -> State {
    let mut state = State {
        screen: redis_pane_core::state::View::Monitor,
        link: up(Tk::Armed),
        ..base()
    };
    state.monitor.status = FeedStatus::Open;
    state.monitor.following = true;
    for (at_ms, raw) in [
        monitor_line(
            MONITOR_NOW_MS,
            r#"1339518083.107412 [0 10.0.0.4:51820] "GET" "user:8812:session""#,
        ),
        monitor_line(
            MONITOR_NOW_MS + 1,
            r#"1339518083.208511 [0 10.0.0.9:33012] "SET" "lock:checkout:8812" "1""#,
        ),
        monitor_line(
            MONITOR_NOW_MS + 2,
            r#"1339518083.309876 [0 10.0.0.4:51820] "EXPIRE" "user:8812:session" "1800""#,
        ),
    ] {
        state.monitor.push_monitor_line(at_ms, raw);
    }
    state
}

#[test]
fn golden_monitor_80_live_tail() {
    assert_golden(
        "monitor_80",
        &draw_at(&monitor_with_lines(), 80, 24, &FixedClock(MONITOR_NOW_MS)),
    );
}

#[test]
fn golden_monitor_130_live_tail() {
    assert_golden(
        "monitor_130",
        &draw_at(&monitor_with_lines(), 130, 26, &FixedClock(MONITOR_NOW_MS)),
    );
}

/// Decision 10: the warning banner survives all the way down to the
/// single-pane floor — never one of the things narrowing sheds.
#[test]
fn the_monitor_banner_survives_at_single_pane_width() {
    let text = draw_at(&monitor_with_lines(), 60, 24, &FixedClock(MONITOR_NOW_MS));
    assert!(
        text.lines().nth(2).is_some_and(|l| l.contains("MONITOR")),
        "{text}"
    );
}

/// The status bar's scan readout, sort and `Esc cancel` describe the key
/// browser; off it they would describe a list the reader cannot see (and
/// `Esc` would do something else). Only the app's own notice follows.
#[test]
fn the_status_bar_leaves_the_key_browser_behind_in_another_view() {
    let mut state = monitor_with_lines();
    state.scan = redis_pane_core::state::ScanState::Running {
        scanned: 41_203,
        estimated_total: 180_000,
    };
    let text = draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS));
    assert!(!text.contains("scanning"), "{text}");
    assert!(!text.contains("cancel"), "{text}");

    state.screen = redis_pane_core::state::View::Keys;
    let text = draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS));
    assert!(text.contains("scanning 41,203"), "{text}");
}

#[test]
fn golden_monitor_paused() {
    let mut state = monitor_with_lines();
    state.monitor.toggle_pause();
    for i in 0..340 {
        state
            .monitor
            .push_monitor_line(MONITOR_NOW_MS + 3 + i, format!("dropped {i}"));
    }
    assert_golden(
        "monitor_paused_80",
        &draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS)),
    );
}

#[test]
fn golden_monitor_filtered() {
    let mut state = monitor_with_lines();
    state.monitor.filter = "SET".to_string();
    assert_golden(
        "monitor_filtered_80",
        &draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS)),
    );
}

#[test]
fn golden_monitor_empty_connecting() {
    let mut state = State {
        screen: redis_pane_core::state::View::Monitor,
        link: up(Tk::Armed),
        ..base()
    };
    state.monitor.status = FeedStatus::Connecting;
    state.monitor.following = true;
    assert!(state.monitor.is_empty());
    assert_golden(
        "monitor_connecting_80",
        &draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS)),
    );
}

#[test]
fn golden_monitor_feed_closed() {
    let mut state = monitor_with_lines();
    state.monitor.status = FeedStatus::Closed {
        reason: Some("the feed connection closed".to_string()),
    };
    assert_golden(
        "monitor_closed_80",
        &draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS)),
    );
}

/// Decision 3: `g m` in `prod`/`unknown` stages this instead of opening —
/// drawn over whatever screen it was staged from (`state.screen` is still
/// `View::Keys` here, since nothing has opened yet).
#[test]
fn golden_monitor_prod_confirm() {
    let mut state = State {
        connection: Connection {
            environment: Environment::Prod,
            ..base().connection
        },
        ..base()
    };
    state.pending_feed = Some(FeedKindMsg::Monitor);
    assert_golden(
        "monitor_prod_confirm_80",
        &draw_at(&state, 80, 24, &FixedClock(MONITOR_NOW_MS)),
    );
}

#[test]
fn golden_help_in_monitor_view() {
    let mut state = monitor_with_lines();
    state.help = Some(HelpView { pane: state.focus });
    let clock = FixedClock(MONITOR_NOW_MS);
    assert_golden("help_monitor_80", &draw_at(&state, 80, 24, &clock));
    assert_golden("help_monitor_130", &draw_at(&state, 130, 24, &clock));
}

// ── Pub/Sub (`g p`, M3 task 5, `docs/plans/m3-pubsub.md`) ──────────────────

use redis_pane_core::state::{PubSubFocus, Subscription};

const PUBSUB_NOW_MS: u64 = 1_339_518_083_500;

fn pubsub_message(
    at_ms: u64,
    channel: &[u8],
    via: Option<&[u8]>,
    payload: &[u8],
) -> (u64, Vec<u8>, Option<Vec<u8>>, Vec<u8>) {
    (
        at_ms,
        channel.to_vec(),
        via.map(|v| v.to_vec()),
        payload.to_vec(),
    )
}

/// An open feed, two subscriptions (one channel, one pattern), and three
/// messages across both channels — the shape most Pub/Sub goldens start
/// from, and the direct golden-frame proof of PLAN's "distinct from
/// Monitor's layout" clause: multiple channels in one tail, each message
/// carrying its own channel identity, which a `MONITOR` line has no
/// analogue of.
fn pubsub_with_messages() -> State {
    let mut state = State {
        screen: redis_pane_core::state::View::PubSub,
        link: up(Tk::Armed),
        ..base()
    };
    state.pubsub.subscriptions = vec![
        Subscription::Channel("orders".into()),
        Subscription::Pattern("user:*".into()),
    ];
    state.pubsub.status = FeedStatus::Open;
    state.pubsub.following = true;
    for (at_ms, channel, via, payload) in [
        pubsub_message(
            PUBSUB_NOW_MS,
            b"orders",
            None,
            br#"{"id":8812,"total":41.5}"#,
        ),
        pubsub_message(PUBSUB_NOW_MS + 1, b"user:42", Some(b"user:*"), b"login"),
        pubsub_message(PUBSUB_NOW_MS + 2, b"orders", None, br#"{"id":8813}"#),
    ] {
        state
            .pubsub
            .push_pubsub_message(at_ms, channel, via, payload);
    }
    state
}

#[test]
fn golden_pubsub_80_multi_channel_tail() {
    assert_golden(
        "pubsub_80",
        &draw_at(&pubsub_with_messages(), 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
}

#[test]
fn golden_pubsub_130_multi_channel_tail() {
    assert_golden(
        "pubsub_130",
        &draw_at(&pubsub_with_messages(), 130, 26, &FixedClock(PUBSUB_NOW_MS)),
    );
}

#[test]
fn golden_pubsub_strip_focused() {
    let mut state = pubsub_with_messages();
    state.pubsub.focus = PubSubFocus::Strip;
    state.pubsub.selected_chip = 1;
    assert_golden(
        "pubsub_strip_focused_80",
        &draw_at(&state, 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
    assert_golden(
        "pubsub_strip_focused_130",
        &draw_at(&state, 130, 26, &FixedClock(PUBSUB_NOW_MS)),
    );
}

#[test]
fn golden_pubsub_empty_with_input_focused() {
    let state = State {
        screen: redis_pane_core::state::View::PubSub,
        link: up(Tk::Armed),
        ..base()
    };
    // Decision 6: `g p` with no remembered subscriptions opens with the
    // add-input focused and dials nothing — this is what `open_pubsub`
    // (`update::pubsub`) actually leaves `State` in, built directly here
    // since this golden is about the render, not the transition into it.
    let mut state = state;
    state.pubsub.adding = true;
    for c in "user:*".chars() {
        state.pubsub.input.push(c);
    }
    assert_golden(
        "pubsub_empty_input_focused_80",
        &draw_at(&state, 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
    assert_golden(
        "pubsub_empty_input_focused_130",
        &draw_at(&state, 130, 26, &FixedClock(PUBSUB_NOW_MS)),
    );
}

#[test]
fn golden_pubsub_paused() {
    let mut state = pubsub_with_messages();
    state.pubsub.focus = PubSubFocus::Tail;
    state.pubsub.toggle_pause();
    for i in 0..12 {
        state.pubsub.push_pubsub_message(
            PUBSUB_NOW_MS + 3 + i,
            b"orders".to_vec(),
            None,
            format!("dropped {i}").into_bytes(),
        );
    }
    assert_golden(
        "pubsub_paused_80",
        &draw_at(&state, 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
}

/// The detail strip pretty-prints a JSON payload — the selected message
/// (the newest, `orders` · `{"id":8813}`) parses as JSON, so the detail
/// strip shows it formatted rather than as one raw line.
#[test]
fn golden_pubsub_json_detail() {
    let state = pubsub_with_messages();
    assert_golden(
        "pubsub_json_detail_80",
        &draw_at(&state, 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
}

#[test]
fn golden_pubsub_closed() {
    let mut state = pubsub_with_messages();
    state.pubsub.status = FeedStatus::Closed {
        reason: Some("the feed connection closed".to_string()),
    };
    assert_golden(
        "pubsub_closed_80",
        &draw_at(&state, 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
}

#[test]
fn golden_help_in_pubsub_view() {
    let mut state = pubsub_with_messages();
    state.help = Some(HelpView { pane: state.focus });
    let clock = FixedClock(PUBSUB_NOW_MS);
    assert_golden("help_pubsub_tail_80", &draw_at(&state, 80, 24, &clock));
    state.pubsub.focus = PubSubFocus::Strip;
    state.help = Some(HelpView { pane: state.focus });
    assert_golden("help_pubsub_strip_80", &draw_at(&state, 80, 24, &clock));
}

#[test]
fn golden_help_pubsub_adding() {
    let mut state = State {
        screen: redis_pane_core::state::View::PubSub,
        link: up(Tk::Armed),
        ..base()
    };
    state.pubsub.adding = true;
    state.help = Some(HelpView { pane: state.focus });
    assert_golden(
        "help_pubsub_adding_80",
        &draw_at(&state, 80, 24, &FixedClock(PUBSUB_NOW_MS)),
    );
}

// ── M5 task 9 — sharded Pub/Sub (`SSUBSCRIBE`, R6.2, ADR-0022) ──────────────

/// `pubsub_with_messages` plus a sharded `orders` subscription and a sharded
/// message on the same channel name as a classic one, newest and selected, so
/// the feed marker and the detail strip both show.
fn pubsub_with_sharded() -> State {
    let mut state = pubsub_with_messages();
    state
        .pubsub
        .subscriptions
        .push(Subscription::Sharded("orders".into()));
    state.pubsub.push_message(
        PUBSUB_NOW_MS + 3,
        b"orders".to_vec(),
        None,
        br#"{"id":8814,"shard":true}"#.to_vec(),
        true,
    );
    state
}

#[test]
fn golden_pubsub_sharded_chip_and_message() {
    let state = pubsub_with_sharded();
    let clock = FixedClock(PUBSUB_NOW_MS);
    let (uni, ascii) = draw_both(&state, 80, 24, &clock);
    assert_golden("pubsub_sharded_80", &uni);
    assert_golden("pubsub_sharded_80_ascii", &ascii);
    // A narrow frame folds the channel into the payload: the marker survives.
    assert_golden("pubsub_sharded_60", &draw_at(&state, 60, 20, &clock));
}

#[test]
fn golden_pubsub_add_form_with_the_sharded_toggle() {
    let mut state = State {
        screen: redis_pane_core::state::View::PubSub,
        link: up(Tk::Armed),
        ..base()
    };
    state.pubsub.adding = true;
    state.pubsub.input.push_str("orders");
    let clock = FixedClock(PUBSUB_NOW_MS);
    assert_golden("pubsub_add_form_off_80", &draw_at(&state, 80, 24, &clock));
    state.pubsub.sharded = true;
    let (uni, ascii) = draw_both(&state, 80, 24, &clock);
    assert_golden("pubsub_add_form_sharded_80", &uni);
    assert_golden("pubsub_add_form_sharded_80_ascii", &ascii);
}

#[test]
fn golden_pubsub_add_form_sharded_disabled_on_redis_6() {
    let mut state = State {
        screen: redis_pane_core::state::View::PubSub,
        link: Link::Up {
            version: "6.2.14".into(),
            tracking: Tk::Armed,
        },
        ..base()
    };
    state.pubsub.adding = true;
    state.pubsub.input.push_str("orders");
    let clock = FixedClock(PUBSUB_NOW_MS);
    assert_golden(
        "pubsub_add_form_redis6_80",
        &draw_at(&state, 80, 24, &clock),
    );
    state.help = Some(HelpView { pane: state.focus });
    assert_golden(
        "help_pubsub_adding_redis6_80",
        &draw_at(&state, 80, 24, &clock),
    );
}

// ── M3 task 6 — the Dashboard (`g d`, R6.3, `docs/plans/m3-dashboard.md`) ───

const DASHBOARD_NOW_MS: u64 = 200_000;

/// A real-looking, healthy Redis 7/8 `INFO` reply — every tile has
/// something to show, and nothing here is close to an alarm threshold.
/// `ops` is the one field that varies between polls (`dashboard_state`'s own
/// synthetic history below) — every other field stays flat, so nothing but
/// the ops/sec sparkline moves across those polls.
fn dashboard_healthy_info(ops: u64) -> redis_pane_core::state::RawInfo {
    redis_pane_core::state::RawInfo::new(vec![
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
                ("used_memory".to_string(), "536870912".to_string()), // 512 MB
                ("used_memory_peak".to_string(), "629145600".to_string()), // 600 MB
                ("maxmemory".to_string(), "2147483648".to_string()),  // 2 GB
            ],
        ),
        (
            "Stats".to_string(),
            vec![
                ("instantaneous_ops_per_sec".to_string(), ops.to_string()),
                ("keyspace_hits".to_string(), "90000".to_string()),
                ("keyspace_misses".to_string(), "10000".to_string()),
                ("evicted_keys".to_string(), "0".to_string()),
                ("expired_keys".to_string(), "128".to_string()),
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

/// A Dashboard with a realistic ops/sec history behind it — 48 synthetic
/// polls of varying traffic (a simple deterministic wave, not `rand`: this
/// crate carries no RNG dependency, and golden fixtures have to be
/// reproducible bit-for-bit anyway) before the final, "real" poll every
/// other tile's figures are asserted against. Without this the sparkline
/// tile would render as a single flat bar — technically correct, but not
/// what the grid actually looks like once a session has been open a
/// while, which is the point of a grid golden.
fn dashboard_state() -> State {
    let mut state = State {
        screen: redis_pane_core::state::View::Dashboard,
        link: up(Tk::Armed),
        ..base()
    };
    for i in 0..48u64 {
        // A triangle wave between ~50 and ~450 ops/sec — enough variation
        // that every sparkline level (`▁` through `█`) is reachable, and
        // deterministic so the fixture never flakes.
        let phase = i % 24;
        let ops = if phase < 12 {
            50 + phase * 33
        } else {
            50 + (24 - phase) * 33
        };
        let at_ms = (DASHBOARD_NOW_MS - 8_000).saturating_sub((48 - i) * 2_000);
        state
            .dashboard
            .record_poll(dashboard_healthy_info(ops), at_ms);
    }
    state
        .dashboard
        .record_poll(dashboard_healthy_info(340), DASHBOARD_NOW_MS - 8_000);
    state
}

fn draw_dashboard_at(state: &State, w: u16, h: u16) -> String {
    let buf = render::frame(
        state,
        &Theme::new(ColorDepth::Monochrome),
        &FixedClock(DASHBOARD_NOW_MS),
        Rect::new(0, 0, w, h),
    );
    render::to_text(&buf)
}

#[test]
fn golden_dashboard_grid_130() {
    assert_golden(
        "dashboard_grid_130",
        &draw_dashboard_at(&dashboard_state(), 130, 26),
    );
}

#[test]
fn golden_dashboard_grid_100() {
    assert_golden(
        "dashboard_grid_100",
        &draw_dashboard_at(&dashboard_state(), 100, 24),
    );
}

#[test]
fn golden_dashboard_grid_80() {
    assert_golden(
        "dashboard_grid_80",
        &draw_dashboard_at(&dashboard_state(), 80, 24),
    );
}

/// Below 80 columns the grid drops to one tile per row and scrolls
/// (decision 5) — the focused tile (`Memory`, the grid's default focus)
/// must still be the first thing on screen.
#[test]
fn golden_dashboard_grid_60() {
    assert_golden(
        "dashboard_grid_60",
        &draw_dashboard_at(&dashboard_state(), 60, 24),
    );
}

/// One tile in `Warn` (hit ratio, just under 80% with enough samples to
/// alarm) and one in `Danger` (memory past 95% of `maxmemory`) — proving
/// "anything alarming is colored" actually reaches the grid, through the
/// semantic `Token::Warn`/`Token::Danger` tokens, never a literal colour.
#[test]
fn golden_dashboard_alarming_tiles() {
    let mut state = State {
        screen: redis_pane_core::state::View::Dashboard,
        link: up(Tk::Armed),
        ..base()
    };
    let info = redis_pane_core::state::RawInfo::new(vec![
        (
            "Memory".to_string(),
            vec![
                ("used_memory".to_string(), "990000000".to_string()),
                ("used_memory_peak".to_string(), "990000000".to_string()),
                ("maxmemory".to_string(), "1000000000".to_string()), // 99% used: Danger
            ],
        ),
        (
            "Stats".to_string(),
            vec![
                ("keyspace_hits".to_string(), "700".to_string()),
                ("keyspace_misses".to_string(), "300".to_string()), // 70%, >= 1000 samples: Warn
            ],
        ),
    ]);
    state.dashboard.record_poll(info, DASHBOARD_NOW_MS - 2_000);
    assert_golden(
        "dashboard_alarming_tiles_100",
        &draw_dashboard_at(&state, 100, 24),
    );
}

/// `Enter` on the focused tile expands its raw `INFO` section (decision 6).
#[test]
fn golden_dashboard_overlay() {
    let mut state = dashboard_state();
    state.dashboard.focused_tile = redis_pane_core::state::TileId::Replication;
    state.dashboard.open_overlay();
    assert_golden("dashboard_overlay_100", &draw_dashboard_at(&state, 100, 24));
}

/// Right after `g d`, before the first `Msg::ServerInfoLoaded` lands — "no
/// blank frame" is a claim about the fetch being issued immediately, not
/// about what is on screen the instant before it answers, so this still has
/// to say something (decision 1).
#[test]
fn golden_dashboard_loading() {
    let mut state = State {
        screen: redis_pane_core::state::View::Dashboard,
        link: up(Tk::Armed),
        ..base()
    };
    state.dashboard.loading = true;
    assert_golden("dashboard_loading_80", &draw_dashboard_at(&state, 80, 24));
}

/// A poll fails after at least one had already landed: the last good tiles
/// stay on screen with their age, and an error banner names what failed —
/// never silently stale (decision 7).
#[test]
fn golden_dashboard_error_with_stale_values() {
    let mut state = dashboard_state();
    state.dashboard.loading = false;
    state.dashboard.error =
        Some("READONLY You can't write against a read only replica".to_string());
    assert_golden(
        "dashboard_error_stale_80",
        &draw_dashboard_at(&state, 80, 24),
    );
}

// ── M4 task 6 — the ASCII glyph set ─────────────────────────────────────────

use redis_pane_core::glyphs::GlyphSet;

/// The same frame drawn with the Unicode and the ASCII glyph set.
fn draw_both(state: &State, w: u16, h: u16, clock: &dyn Clock) -> (String, String) {
    let at = |set| {
        let buf = render::frame(
            state,
            &Theme::new(ColorDepth::Monochrome).with_glyphs(set),
            clock,
            Rect::new(0, 0, w, h),
        );
        render::to_text(&buf)
    };
    (at(GlyphSet::Unicode), at(GlyphSet::Ascii))
}

fn ascii_frame(state: &State, w: u16, h: u16, clock: &dyn Clock) -> String {
    draw_both(state, w, h, clock).1
}

fn help_state() -> State {
    State {
        link: up(Tk::Armed),
        help: Some(HelpView {
            pane: redis_pane_core::render::layout::Pane::Keys,
        }),
        rows: 14,
        ..base()
    }
}

// ── M4 task 5: light and high-contrast, which ADR-0011 promised ─────────────

use redis_pane_core::theme::Palette;

/// The keys pane in a named built-in theme: the selection bar, every type
/// colour, the Environment band and the Muted metadata in one frame.
fn browser_in(palette: Palette, depth: ColorDepth) -> String {
    let frame = render::frame(
        &many_keys(),
        &Theme::with_palette(depth, &palette),
        &CLOCK,
        Rect::new(0, 0, 90, 12),
    );
    render::to_golden(&frame)
}

#[test]
fn golden_browser_style_light_truecolor() {
    assert_golden(
        "browser_style_light_truecolor",
        &browser_in(Palette::light(), ColorDepth::TrueColor),
    );
}

#[test]
fn golden_browser_style_high_contrast_truecolor() {
    assert_golden(
        "browser_style_high_contrast_truecolor",
        &browser_in(Palette::high_contrast(), ColorDepth::TrueColor),
    );
}

#[test]
fn golden_browser_style_light_ansi256() {
    assert_golden(
        "browser_style_light_ansi256",
        &browser_in(Palette::light(), ColorDepth::Ansi256),
    );
}

#[test]
fn a_theme_changes_the_styles_but_never_the_text() {
    let text_of = |g: &str| g.split("--- styles ---").next().unwrap().to_string();
    let dark = browser_in(Palette::dark(), ColorDepth::TrueColor);
    for palette in [Palette::light(), Palette::high_contrast()] {
        let name = palette.name.clone();
        let other = browser_in(palette, ColorDepth::TrueColor);
        assert_ne!(dark, other, "{name} must restyle the frame");
        assert_eq!(
            text_of(&dark),
            text_of(&other),
            "{name} must not move a cell"
        );
    }
}

#[test]
fn golden_browser_130_ascii() {
    assert_golden(
        "browser_130_ascii",
        &ascii_frame(&browsing(), 130, 26, &CLOCK),
    );
}

#[test]
fn golden_viewer_detached_ascii() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.view.selected = 0;
    assert_golden(
        "viewer_detached_ascii",
        &ascii_frame(&state, 130, 22, &CLOCK),
    );
}

#[test]
fn golden_dashboard_grid_130_ascii() {
    assert_golden(
        "dashboard_grid_130_ascii",
        &ascii_frame(&dashboard_state(), 130, 26, &FixedClock(DASHBOARD_NOW_MS)),
    );
}

#[test]
fn golden_help_overlay_ascii() {
    assert_golden(
        "help_overlay_ascii",
        &ascii_frame(&help_state(), 100, 14, &CLOCK),
    );
}

#[test]
fn golden_slowlog_130_ascii() {
    assert_golden(
        "slowlog_130_ascii",
        &ascii_frame(
            &slowlog_with_entries(),
            130,
            26,
            &FixedClock(SLOWLOG_NOW_MS),
        ),
    );
}

/// The claim DESIGN §5 makes: choosing ASCII does not move anything. Every row
/// of the ASCII frame is the same width as the Unicode frame's, and none of it
/// is outside ASCII — which also proves no draw site was left holding a
/// literal glyph.
#[test]
fn the_ascii_frame_has_the_unicode_frames_layout_and_no_non_ascii() {
    use ratatui::text::Line;
    let mut detached = opened("user:8812:session", hash_value(), 2_537);
    detached.view.selected = 0;
    type Fixture = (&'static str, State, u16, u16, Box<dyn Clock>);
    let fixtures: Vec<Fixture> = vec![
        ("browser", browsing(), 130, 26, Box::new(CLOCK)),
        ("browser narrow", browsing(), 70, 20, Box::new(CLOCK)),
        ("gone key", with_a_gone_key(), 130, 26, Box::new(CLOCK)),
        ("pending", pending(), 130, 26, Box::new(CLOCK)),
        (
            "tree",
            {
                let mut s = many_keys();
                s.tree_mode = true;
                s.rebuild_list();
                s
            },
            100,
            24,
            Box::new(CLOCK),
        ),
        (
            "hash open",
            opened("user:8812:session", hash_value(), 2_537),
            130,
            22,
            Box::new(CLOCK),
        ),
        ("detached", detached, 130, 22, Box::new(CLOCK)),
        ("help", help_state(), 100, 14, Box::new(CLOCK)),
        (
            "dashboard",
            dashboard_state(),
            130,
            26,
            Box::new(FixedClock(DASHBOARD_NOW_MS)),
        ),
        (
            "slowlog",
            slowlog_with_entries(),
            130,
            26,
            Box::new(FixedClock(SLOWLOG_NOW_MS)),
        ),
        (
            "sharded pub/sub",
            pubsub_with_sharded(),
            100,
            24,
            Box::new(FixedClock(PUBSUB_NOW_MS)),
        ),
        (
            "monitor",
            monitor_with_lines(),
            130,
            26,
            Box::new(FixedClock(MONITOR_NOW_MS)),
        ),
    ];
    for (name, state, w, h, clock) in &fixtures {
        let (uni, ascii) = draw_both(state, *w, *h, clock.as_ref());
        assert_eq!(uni.lines().count(), ascii.lines().count(), "{name}");
        for (u, a) in uni.lines().zip(ascii.lines()) {
            assert_eq!(
                Line::from(u).width(),
                Line::from(a).width(),
                "{name}: layout moved\n  unicode | {u}\n  ascii   | {a}"
            );
            assert!(a.is_ascii(), "{name}: a glyph escaped the table: {a}");
        }
    }
}

#[test]
fn monochrome_frames_are_identical_in_every_theme() {
    let dark = browser_in(Palette::dark(), ColorDepth::Monochrome);
    for palette in [Palette::light(), Palette::high_contrast()] {
        assert_eq!(dark, browser_in(palette, ColorDepth::Monochrome));
    }
}

// ── M4 follow-up: themes paint their own background ─────────────────────────

/// Every kind of overlay the frame can draw, each over a screen: help over the
/// browser, a mutation confirm over Slowlog, a feed confirm over the browser,
/// and a plain browser and Slowlog with nothing open.
fn overlay_states() -> Vec<(&'static str, State, u64)> {
    let mut help = many_keys();
    help.help = Some(HelpView { pane: help.focus });
    let mut confirm = slowlog_with_entries();
    confirm.confirm = Some(PendingMutation::ResetSlowlog);
    let feed = State {
        pending_feed: Some(FeedKindMsg::Monitor),
        connection: Connection {
            environment: Environment::Prod,
            ..base().connection
        },
        ..base()
    };
    vec![
        ("browser", many_keys(), 0),
        ("help", help, 0),
        ("slowlog", slowlog_with_entries(), SLOWLOG_NOW_MS),
        ("confirm", confirm, SLOWLOG_NOW_MS),
        ("feed confirm", feed, MONITOR_NOW_MS),
    ]
}

#[test]
fn in_light_no_cell_of_any_frame_has_an_unpainted_background() {
    use ratatui::style::Color;
    for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
        let theme = Theme::with_palette(depth, &Palette::light());
        let ground = theme.background().expect("light paints a background");
        for (what, state, now) in overlay_states() {
            for (w, h) in [(100, 24), (80, 24), (60, 20)] {
                let buf = render::frame(&state, &theme, &FixedClock(now), Rect::new(0, 0, w, h));
                for y in 0..h {
                    for x in 0..w {
                        let bg = buf.cell((x, y)).unwrap().bg;
                        assert!(
                            bg != Color::Reset,
                            "{what} {depth:?} {w}x{h}: hole at ({x},{y})"
                        );
                    }
                }
                // And the ground really is what shows through behind the
                // title bar's rule, not merely "something".
                assert_eq!(buf.cell((0, 0)).unwrap().bg, ground, "{what} {depth:?}");
            }
        }
    }
}

#[test]
fn high_contrast_paints_every_cell_too() {
    use ratatui::style::Color;
    let theme = Theme::with_palette(ColorDepth::TrueColor, &Palette::high_contrast());
    for (what, state, now) in overlay_states() {
        let buf = render::frame(&state, &theme, &FixedClock(now), Rect::new(0, 0, 90, 24));
        assert!(
            buf.content().iter().all(|c| c.bg != Color::Reset),
            "{what}: a cell is unpainted"
        );
    }
}

#[test]
fn text_in_a_painting_theme_never_falls_back_to_the_terminal_foreground() {
    use ratatui::style::Color;
    let theme = Theme::with_palette(ColorDepth::TrueColor, &Palette::light());
    for (what, state, now) in overlay_states() {
        let buf = render::frame(&state, &theme, &FixedClock(now), Rect::new(0, 0, 100, 24));
        for (i, c) in buf.content().iter().enumerate() {
            assert!(
                c.symbol().trim().is_empty() || c.fg != Color::Reset,
                "{what}: `{}` at cell {i} would draw in the terminal's own foreground",
                c.symbol()
            );
        }
    }
}

#[test]
fn an_overlay_keeps_the_selection_and_wash_backgrounds_it_was_given() {
    use redis_pane_core::theme::Token;
    let theme = Theme::with_palette(ColorDepth::TrueColor, &Palette::light());
    let buf = render::frame(&many_keys(), &theme, &CLOCK, Rect::new(0, 0, 90, 12));
    let selected = theme.style(Token::Selected).bg;
    assert!(
        buf.content().iter().any(|c| Some(c.bg) == selected),
        "the selection bar is still its own colour"
    );
}

#[test]
fn dark_and_monochrome_paint_nothing_under_any_overlay() {
    use ratatui::style::Color;
    for theme in [
        Theme::new(ColorDepth::TrueColor),
        Theme::new(ColorDepth::Ansi256),
        Theme::with_palette(ColorDepth::Monochrome, &Palette::light()),
        Theme::with_palette(ColorDepth::Monochrome, &Palette::high_contrast()),
    ] {
        assert_eq!(theme.background(), None);
        for (what, state, now) in overlay_states() {
            let buf = render::frame(&state, &theme, &FixedClock(now), Rect::new(0, 0, 90, 24));
            // Some cell must still be the terminal's own, or this would pass
            // for a theme that painted everything.
            assert!(
                buf.content().iter().any(|c| c.bg == Color::Reset),
                "{what}: nothing is left to the terminal"
            );
        }
    }
}

#[test]
fn background_terminal_on_a_light_base_paints_nothing() {
    use ratatui::style::Color;
    let cfg = redis_pane_core::config::parse(
        r#"{"theme":"m","themes":{"m":{"base":"light","background":"terminal"}}}"#,
    )
    .unwrap();
    let palette = redis_pane_core::theme::select(None, Some(&cfg)).unwrap();
    let theme = Theme::with_palette(ColorDepth::TrueColor, &palette);
    let buf = render::frame(&many_keys(), &theme, &CLOCK, Rect::new(0, 0, 90, 12));
    assert!(buf.content().iter().any(|c| c.bg == Color::Reset));
}

/// An overlay open in `light`: the help overlay's box and everything around it
/// carry the painted ground, with no dark rectangle where the overlay blanked
/// its cells.
#[test]
fn golden_help_overlay_light_truecolor() {
    let mut state = many_keys();
    state.help = Some(HelpView { pane: state.focus });
    let theme = Theme::with_palette(ColorDepth::TrueColor, &Palette::light());
    let buf = render::frame(&state, &theme, &CLOCK, Rect::new(0, 0, 90, 24));
    assert_golden("help_overlay_light_truecolor", &render::to_golden(&buf));
}

// ── M5 task 2 — the Cluster's shape in the title bar (ADR-0022) ─────────────

fn cluster_state(target: &str) -> State {
    State {
        cols: 140,
        rows: 3,
        connection: Connection {
            target: target.into(),
            environment: Environment::Staging,
            source: Source::Profile("staging".into()),
            topology: Some(redis_pane_core::state::Topology {
                primaries: 3,
                nodes: 6,
            }),
        },
        link: up(Tk::Armed),
        ..State::default()
    }
}

fn title_at(state: &State, w: u16, set: GlyphSet) -> String {
    let buf = render::frame(
        state,
        &Theme::new(ColorDepth::Monochrome).with_glyphs(set),
        &CLOCK,
        Rect::new(0, 0, w, 3),
    );
    render::to_text(&buf).lines().next().unwrap().to_string()
}

#[test]
fn golden_title_bar_cluster() {
    let state = cluster_state("cache-01:7000/0");
    let long = cluster_state("redis-cluster://cache-01.eu-west-1.example.com:7000/0");
    let mut rendered = Vec::new();
    for w in [140u16, 120, 100, 80, 60] {
        rendered.push(format!(
            "{w:>3} | {}",
            title_at(&state, w, GlyphSet::Unicode)
        ));
    }
    rendered.push(format!(
        "{:>3} | {}",
        "asc",
        title_at(&state, 140, GlyphSet::Ascii)
    ));
    for w in [140u16, 80, 60] {
        rendered.push(format!(
            "{w:>3} | {}",
            title_at(&long, w, GlyphSet::Unicode)
        ));
    }
    assert_golden("title_bar_cluster", &rendered.join("\n"));
}

#[test]
fn the_cluster_segment_yields_before_the_target_and_never_costs_environment_or_source() {
    let state = cluster_state("cache-01:7000/0");
    let wide = title_at(&state, 140, GlyphSet::Unicode);
    assert!(
        wide.contains("cache-01:7000 · cluster · 3 primaries · 6 nodes"),
        "{wide}"
    );
    assert!(!wide.contains("/0"), "a Cluster shows no db: {wide}");
    for w in 50..=140u16 {
        let title = title_at(&state, w, GlyphSet::Unicode);
        assert!(
            title.contains("staging"),
            "Environment lost at {w}: {title}"
        );
        assert!(
            title.contains("from profile staging"),
            "Source lost at {w}: {title}"
        );
        if title.contains("cluster") {
            assert!(
                title.contains("cache-01:7000"),
                "the segment outlived the whole target at {w}: {title}"
            );
        }
    }
}

#[test]
fn the_cluster_segment_uses_the_ascii_separator_and_keeps_its_width() {
    let state = cluster_state("cache-01:7000/0");
    let uni = title_at(&state, 140, GlyphSet::Unicode);
    let ascii = title_at(&state, 140, GlyphSet::Ascii);
    assert!(ascii.is_ascii(), "{ascii}");
    assert!(ascii.contains("cluster"), "{ascii}");
    assert_eq!(uni.chars().count(), ascii.chars().count());
}

#[test]
fn a_single_node_target_is_unchanged_by_the_cluster_work() {
    let mut state = cluster_state("cache-01:7000/0");
    state.connection.topology = None;
    let title = title_at(&state, 140, GlyphSet::Unicode);
    assert!(title.contains("cache-01:7000/0"), "{title}");
    assert!(!title.contains("cluster"), "{title}");
}

#[test]
fn copying_the_command_on_a_cluster_updates_through_the_core() {
    let mut state = opened("k", hash_value(), 600);
    state.connection.target = "cache-01:7000/0".into();
    state.connection.topology = Some(redis_pane_core::state::Topology {
        primaries: 3,
        nodes: 6,
    });
    let open = state.open.as_ref().unwrap();
    let cmd = redis_cli_command(
        &state.connection.target,
        &open.name,
        open.value.as_ref().unwrap(),
        true,
    );
    assert_eq!(cmd, "redis-cli -c -h cache-01 -p 7000 HGETALL k");
}

// ── M5 task 8 — Slowlog and Monitor on a Cluster ────────────────────────────

fn cluster_view(view: redis_pane_core::state::View, w: u16) -> State {
    let mut state = cluster_state("redis-cluster://cache-01:7000/0");
    state.cols = w;
    state.rows = 24;
    state.screen = view;
    state
}

fn node_entry(
    node: &str,
    id: i64,
    secs_ago: i64,
    duration_us: i64,
    command: &str,
) -> redis_pane_core::state::SlowlogEntry {
    let mut e = slowlog_entry(
        id,
        SLOWLOG_NOW_S - secs_ago,
        duration_us,
        command,
        "10.0.0.4:51820",
        "worker-3",
    );
    e.node = node.into();
    e
}

/// A merged slowlog from three nodes, ids restarting per node (so `(node, id)`
/// is the identity), the same id on two of them.
fn cluster_slowlog(w: u16) -> State {
    let mut state = cluster_view(redis_pane_core::state::View::Slowlog, w);
    state.slowlog.set_entries(vec![
        node_entry("10.0.0.1:7000", 4, 5, 185_000, "HGETALL user:8812:session"),
        node_entry("10.0.0.2:7001", 4, 9, 2_400, "ZRANGE leaderboard 0 -1"),
        node_entry("10.0.0.1:7000", 3, 20, 12_300, "SMEMBERS tags"),
        node_entry("10.0.0.3:7002", 1, 31, 1_250_000, "KEYS user:*"),
    ]);
    state
}

fn cluster_draw(state: &State, w: u16, h: u16) -> String {
    draw_at(state, w, h, &FixedClock(SLOWLOG_NOW_MS))
}

#[test]
fn golden_slowlog_on_a_cluster() {
    for w in [140u16, 80, 60] {
        assert_golden(
            &format!("cluster_slowlog_{w}"),
            &cluster_draw(&cluster_slowlog(w), w, 24),
        );
    }
}

#[test]
fn golden_slowlog_on_a_cluster_ascii() {
    assert_golden(
        "cluster_slowlog_ascii",
        &ascii_frame(&cluster_slowlog(100), 100, 24, &FixedClock(SLOWLOG_NOW_MS)),
    );
}

#[test]
fn golden_slowlog_on_a_cluster_with_a_failed_node() {
    let mut state = cluster_slowlog(100);
    state.slowlog.failed = vec![redis_pane_core::state::NodeFailure {
        node: "10.0.0.3:7002".into(),
        detail: "no connection within 4s".into(),
    }];
    state.slowlog.set_entries(
        state
            .slowlog
            .entries()
            .iter()
            .filter(|e| e.node != "10.0.0.3:7002")
            .cloned()
            .collect(),
    );
    assert_golden(
        "cluster_slowlog_failed_node_100",
        &cluster_draw(&state, 100, 24),
    );
}

#[test]
fn golden_slowlog_reset_confirm_names_the_node_count() {
    let mut state = cluster_slowlog(100);
    state.confirm = Some(redis_pane_core::state::PendingMutation::ResetSlowlog);
    let frame = cluster_draw(&state, 100, 24);
    assert!(frame.contains("SLOWLOG RESET on 6 nodes"), "{frame}");
    assert_golden("cluster_slowlog_reset_confirm_100", &frame);
}

fn node_line(at_ms: u64, node: &str, raw: &str) -> (u64, String, Option<String>) {
    (at_ms, raw.to_string(), Some(node.to_string()))
}

fn cluster_monitor(w: u16) -> State {
    let mut state = cluster_view(redis_pane_core::state::View::Monitor, w);
    state.monitor.status = FeedStatus::Open;
    state.monitor.following = true;
    for (at_ms, raw, node) in [
        node_line(
            MONITOR_NOW_MS,
            "10.0.0.1:7000",
            r#"1339518083.107412 [0 10.0.0.4:51820] "GET" "user:8812:session""#,
        ),
        node_line(
            MONITOR_NOW_MS + 1,
            "10.0.0.2:7001",
            r#"1339518083.409003 [0 10.0.0.4:51822] "HSET" "order:77" "state" "paid""#,
        ),
        node_line(
            MONITOR_NOW_MS + 2,
            "10.0.0.3:7002",
            r#"1339518084.002120 [0 10.0.0.9:40112] "EXPIRE" "cart:1" "600""#,
        ),
    ] {
        state.monitor.push_monitor_line_from(at_ms, raw, node);
    }
    state
}

#[test]
fn golden_monitor_on_a_cluster() {
    for w in [140u16, 80, 60] {
        let frame = draw(&cluster_monitor(w), w, 24);
        assert!(frame.contains("MONITOR on 3 primaries"), "{w}: {frame}");
        assert_golden(&format!("cluster_monitor_{w}"), &frame);
    }
}

#[test]
fn golden_monitor_on_a_cluster_ascii() {
    assert_golden(
        "cluster_monitor_ascii",
        &ascii_frame(&cluster_monitor(100), 100, 24, &CLOCK),
    );
}

#[test]
fn golden_monitor_on_a_cluster_with_a_stopped_feed() {
    let mut state = cluster_monitor(100);
    state
        .monitor
        .stopped
        .push(redis_pane_core::state::StoppedNode {
            node: "10.0.0.2:7001".into(),
            reason: "the feed connection closed".into(),
        });
    let frame = draw(&state, 100, 24);
    assert!(frame.contains("MONITOR on 2 of 3 primaries"), "{frame}");
    assert!(frame.contains("10.0.0.2:7001 stopped"), "{frame}");
    assert_golden("cluster_monitor_stopped_feed_100", &frame);
}

#[test]
fn golden_monitor_confirm_on_a_cluster_names_the_count() {
    for (name, env) in [
        ("prod", Environment::Prod),
        ("unknown", Environment::Unknown),
    ] {
        let mut state = cluster_view(redis_pane_core::state::View::Keys, 100);
        state.connection.environment = env;
        state.pending_feed = Some(redis_pane_core::command::FeedKindMsg::Monitor);
        let frame = draw(&state, 100, 24);
        assert!(frame.contains("all 3 primaries"), "{name}: {frame}");
        assert_golden(&format!("cluster_monitor_confirm_{name}_100"), &frame);
    }
}

#[test]
fn golden_help_in_slowlog_and_monitor_on_a_cluster_is_not_dimmed() {
    use redis_pane_core::state::View;
    for (name, view) in [("slowlog", View::Slowlog), ("monitor", View::Monitor)] {
        let mut state = cluster_view(view, 100);
        state.help = Some(HelpView { pane: state.focus });
        assert_golden(&format!("cluster_help_{name}_100"), &draw(&state, 100, 24));
        let rows = help::here(&state, help::context(&state));
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.refused.is_none()), "{name}");
    }
}

// ── M5 task 7 — the Cluster Dashboard ───────────────────────────────────────

/// One node's `INFO`, in the shape the Dashboard reads. `role` is `master` or
/// `slave`; a replica carries its link and lag, a primary its replica count.
fn cluster_node_info(
    role: &str,
    used_mb: u64,
    max_mb: u64,
    ops: u64,
    lag_secs: u64,
) -> redis_pane_core::state::RawInfo {
    let mb = |n: u64| (n * 1024 * 1024).to_string();
    let replication = if role == "master" {
        vec![
            ("role".to_string(), "master".to_string()),
            ("connected_slaves".to_string(), "1".to_string()),
            (
                "slave0".to_string(),
                format!("ip=127.0.0.1,port=7103,state=online,offset=196,lag={lag_secs}"),
            ),
        ]
    } else {
        vec![
            ("role".to_string(), "slave".to_string()),
            (
                "master_last_io_seconds_ago".to_string(),
                lag_secs.to_string(),
            ),
            ("master_link_status".to_string(), "up".to_string()),
        ]
    };
    redis_pane_core::state::RawInfo::new(vec![
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
                ("used_memory".to_string(), mb(used_mb)),
                ("used_memory_peak".to_string(), mb(used_mb + 40)),
                ("maxmemory".to_string(), mb(max_mb)),
            ],
        ),
        (
            "Stats".to_string(),
            vec![
                ("instantaneous_ops_per_sec".to_string(), ops.to_string()),
                ("keyspace_hits".to_string(), "90000".to_string()),
                ("keyspace_misses".to_string(), "10000".to_string()),
                ("evicted_keys".to_string(), "0".to_string()),
                ("expired_keys".to_string(), "128".to_string()),
                ("rejected_connections".to_string(), "0".to_string()),
            ],
        ),
        ("Replication".to_string(), replication),
    ])
}

fn cluster_readings(failed_replica: bool) -> Vec<redis_pane_core::state::NodeReading> {
    use redis_pane_core::state::{NodeReading, NodeRole};
    let primary = |port: u16, used: u64, ops: u64| NodeReading {
        addr: format!("127.0.0.1:{port}"),
        role: NodeRole::Primary,
        slots: if port == 7102 { 5462 } else { 5461 },
        info: Ok(cluster_node_info("master", used, 512, ops, 0)),
    };
    let replica = |port: u16, used: u64, lag: u64| NodeReading {
        addr: format!("127.0.0.1:{port}"),
        role: NodeRole::Replica,
        slots: 0,
        info: Ok(cluster_node_info("slave", used, 512, 3, lag)),
    };
    let mut nodes = vec![
        primary(7100, 200, 120),
        primary(7101, 410, 340),
        primary(7102, 150, 90),
        replica(7103, 200, 0),
        replica(7104, 410, 2),
        replica(7105, 150, 1),
    ];
    if failed_replica {
        nodes[4].info = Err("no answer to INFO within 4s".to_string());
    }
    nodes
}

fn cluster_health_text(state: &str, assigned: u32) -> Result<String, String> {
    Ok(format!(
        "cluster_state:{state}\r\ncluster_slots_assigned:{assigned}\r\ncluster_slots_ok:{assigned}\r\ncluster_slots_pfail:0\r\ncluster_slots_fail:0\r\ncluster_known_nodes:6\r\ncluster_size:3\r\n"
    ))
}

fn cluster_dashboard(failed_replica: bool, health: &str, assigned: u32) -> State {
    let mut state = cluster_state("redis-cluster://127.0.0.1:7100/0");
    state.screen = redis_pane_core::state::View::Dashboard;
    let mut cluster = redis_pane_core::state::ClusterDash::default();
    // A first poll, then the one on screen, so the rising-counter alarms have
    // something to compare against.
    cluster.record(
        cluster_readings(false),
        cluster_health_text("ok", 16384),
        DASHBOARD_NOW_MS - 12_000,
    );
    cluster.record(
        cluster_readings(failed_replica),
        cluster_health_text(health, assigned),
        DASHBOARD_NOW_MS - 8_000,
    );
    state.dashboard.cluster = Some(Box::new(cluster));
    state.dashboard.last_updated_ms = Some(DASHBOARD_NOW_MS - 8_000);
    state
}

fn cluster_dashboard_at(state: &mut State, w: u16, h: u16) -> String {
    state.cols = w;
    state.rows = h;
    draw_dashboard_at(state, w, h)
}

#[test]
fn golden_cluster_dashboard_140() {
    let mut state = cluster_dashboard(false, "ok", 16384);
    assert_golden(
        "cluster_dashboard_overview_140",
        &cluster_dashboard_at(&mut state, 140, 30),
    );
}

#[test]
fn golden_cluster_dashboard_80() {
    let mut state = cluster_dashboard(false, "ok", 16384);
    assert_golden(
        "cluster_dashboard_overview_80",
        &cluster_dashboard_at(&mut state, 80, 24),
    );
}

#[test]
fn golden_cluster_dashboard_60() {
    let mut state = cluster_dashboard(false, "ok", 16384);
    assert_golden(
        "cluster_dashboard_overview_60",
        &cluster_dashboard_at(&mut state, 60, 24),
    );
}

#[test]
fn golden_cluster_dashboard_ascii() {
    let mut state = cluster_dashboard(false, "ok", 16384);
    state.cols = 100;
    state.rows = 30;
    assert_golden(
        "cluster_dashboard_overview_ascii",
        &ascii_frame(&state, 100, 30, &FixedClock(DASHBOARD_NOW_MS)),
    );
}

#[test]
fn golden_cluster_dashboard_a_node_view() {
    let mut state = cluster_dashboard(false, "ok", 16384);
    // The real keys: `j` to the second row, `Enter` to open it.
    let (s, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('j'))));
    let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    state = s;
    assert!(state.dashboard.drilled());
    assert_golden(
        "cluster_dashboard_node_100",
        &cluster_dashboard_at(&mut state, 100, 26),
    );
}

#[test]
fn golden_cluster_dashboard_a_failed_node_row() {
    let mut state = cluster_dashboard(true, "ok", 16384);
    assert_golden(
        "cluster_dashboard_failed_node_100",
        &cluster_dashboard_at(&mut state, 100, 30),
    );
}

#[test]
fn golden_cluster_dashboard_cluster_state_fail() {
    let mut state = cluster_dashboard(false, "fail", 16000);
    assert_golden(
        "cluster_dashboard_state_fail_100",
        &cluster_dashboard_at(&mut state, 100, 30),
    );
}

#[test]
fn golden_cluster_dashboard_before_the_first_poll() {
    let mut state = cluster_state("redis-cluster://127.0.0.1:7100/0");
    state.screen = redis_pane_core::state::View::Dashboard;
    state.dashboard.loading = true;
    assert_golden(
        "cluster_dashboard_loading_80",
        &cluster_dashboard_at(&mut state, 80, 24),
    );
}

// ── M6 task 3: the rebuild job's readout ────────────────────────────────────

/// `many_keys()` with a sort change in flight and a two-key slice, stopped
/// part-way: the old list on screen, the readout saying how far along.
fn rebuilding() -> State {
    let mut state = many_keys();
    state.rebuild_slice = Some(2);
    state.list.sort = SortBy::Name;
    state.rebuild_list_async();
    for _ in 0..4 {
        state.rebuild_step();
    }
    assert!(state.rebuild_running(), "the job must still be running");
    state
}

#[test]
fn golden_rebuilding_readout_140() {
    assert_golden("browser_rebuilding_140", &draw(&rebuilding(), 140, 22));
}

#[test]
fn golden_rebuilding_readout_80() {
    assert_golden("browser_rebuilding_80", &draw(&rebuilding(), 80, 22));
}

#[test]
fn golden_rebuilding_readout_ascii() {
    assert_golden(
        "browser_rebuilding_ascii",
        &ascii_frame(&rebuilding(), 130, 22, &CLOCK),
    );
}

#[test]
fn the_readout_is_gone_once_the_job_swaps() {
    let mut state = rebuilding();
    state.run_rebuild_to_completion();
    assert!(!draw(&state, 140, 22).contains("rebuilding"));
}

#[test]
fn the_readout_is_a_warning_in_colour() {
    let state = rebuilding();
    let buf = render::frame(
        &state,
        &Theme::new(ColorDepth::TrueColor),
        &CLOCK,
        Rect::new(0, 0, 140, 22),
    );
    let theme = Theme::new(ColorDepth::TrueColor);
    let warn = theme.style(redis_pane_core::theme::Token::Warn);
    let text = render::to_text(&buf);
    let (y, line) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("rebuilding"))
        .expect("the readout is drawn");
    let x = line[..line.find("rebuilding").unwrap()].chars().count() as u16;
    assert_eq!(Some(buf[(x, y as u16)].fg), warn.fg);
}

// ── M2 task 11: rename a key (`docs/plans/m2-task11-rename.md`) ─────────────

fn rename_capture_open() -> State {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Keys;
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('R'))));
    assert!(state.rename.is_some(), "R opens the capture");
    state
}

fn rename_typed(mut state: State, backspaces: usize, text: &str) -> State {
    for _ in 0..backspaces {
        state = update(state, Msg::Key(KeyPress::plain(KeyCode::Backspace))).0;
    }
    for c in text.chars() {
        state = update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c)))).0;
    }
    state
}

fn rename_staged(state: State, backspaces: usize, text: &str) -> State {
    let state = rename_typed(state, backspaces, text);
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::RenameKey { .. })
    ));
    state
}

#[test]
fn golden_rename_capture_prefilled_with_the_current_name() {
    assert_golden("rename_capture", &draw(&rename_capture_open(), 130, 22));
}

#[test]
fn golden_rename_capture_with_a_typed_name() {
    let state = rename_typed(rename_capture_open(), 7, "cart");
    assert_golden("rename_capture_typed", &draw(&state, 130, 22));
}

#[test]
fn golden_rename_capture_refuses_an_empty_name_inline() {
    let state = rename_typed(rename_capture_open(), 17, "");
    assert_golden("rename_capture_empty", &draw(&state, 130, 22));
}

#[test]
fn golden_hint_bar_while_capturing_a_new_name() {
    let state = rename_capture_open();
    assert_golden("hint_bar_rename_capture", &hint_bar(&state, 130));
}

#[test]
fn golden_confirm_rename_ok() {
    let state = rename_staged(rename_capture_open(), 7, "cart");
    let (state, _) = update(
        state,
        Msg::TargetChecked {
            key: "user:8812:cart".into(),
            exists: false,
        },
    );
    assert_golden("confirm_rename", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_rename_target_exists() {
    let state = rename_staged(rename_capture_open(), 7, "cart");
    let (state, _) = update(
        state,
        Msg::TargetChecked {
            key: "user:8812:cart".into(),
            exists: true,
        },
    );
    assert_golden("confirm_rename_target_exists", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_rename_cross_slot_on_a_cluster() {
    let mut state = rename_capture_open();
    state.connection.topology = Some(redis_pane_core::state::Topology {
        primaries: 3,
        nodes: 6,
    });
    let state = rename_staged(state, 7, "cart");
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::RenameKey {
            slots: redis_pane_core::state::SlotPath::CrossSlotRefused,
            ..
        })
    ));
    assert_golden("confirm_rename_cross_slot", &draw(&state, 130, 22));
}

#[test]
fn golden_confirm_rename_under_read_only_shows_the_reason() {
    let mut state = rename_capture_open();
    state.read_only = Some(ReadOnlyReason::Environment);
    let state = rename_staged(state, 7, "cart");
    assert_golden("confirm_rename_read_only", &draw(&state, 130, 22));
}

#[test]
fn the_rename_row_is_dimmed_with_the_reason_under_read_only_mode() {
    let mut state = opened("user:8812:session", hash_value(), 2_537);
    state.focus = Pane::Keys;
    let row_of = |state: &State| {
        help::here(state, help::context(state))
            .into_iter()
            .find(|r| r.label == "rename")
            .expect("the keys pane lists rename")
    };
    let free = row_of(&state);
    assert_eq!(free.keys, "R");
    assert!(free.refused.is_none());

    state.read_only = Some(ReadOnlyReason::Environment);
    let dimmed = row_of(&state);
    let refusal = dimmed.refused.expect("dimmed under read-only");
    assert_eq!(refusal.reason, Some(ReadOnlyReason::Environment));
    assert!(refusal.preview_only, "the preview still opens");
    assert_eq!(refusal.text(), "read-only (environment) · preview only");
}
