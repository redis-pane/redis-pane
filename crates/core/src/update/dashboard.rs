//! The Dashboard view (`g d`, R6.3, M3 task 6, `docs/plans/m3-dashboard.md`):
//! opening it, the two replies `Command::FetchServerInfo` can answer with,
//! the poll tick the shell's interval produces, tile-focus movement, the
//! raw-`INFO` overlay, and the manual refetch (`r`)/copy (`c`).
//!
//! What this module does *not* do: no timer lives here. Decision 1 puts the
//! `tokio::time::interval` in the shell (`crates/app/src/terminal.rs`),
//! always ticking; each tick produces `Msg::DashboardPollTick`, and
//! `dashboard_poll_tick` below is the one place that decides whether a tick
//! actually means anything — issuing `Command::FetchServerInfo` only while
//! `state.screen == View::Dashboard`, the main connection is up, and no
//! poll is already in flight. Switching into or out of this view touches
//! exactly `State::screen` and, on the way in, `State::dashboard` — never
//! `State::open`, `State::link`, or anything a Refetch/tracking arming
//! depends on, the same invariant `update/slowlog.rs` documents for its own
//! view.

use super::*;
use crate::command::InfoToken;
use crate::state::{RawInfo, View, dashboard::tiles_per_row};

/// Mint the token for a `FetchServerInfo` about to be issued — the same
/// "only the core invents one" discipline [`issue_read`] follows for
/// [`crate::command::ReadToken`], applied one poll wide (review item (b)):
/// a manual `g d`/`r` can overlap a timer-driven poll, and only the reply
/// carrying the token this just minted is allowed to land
/// (`server_info_loaded`/`server_info_failed`'s own guard).
fn issue_info_token(state: &mut State) -> InfoToken {
    state.dashboard.poll_token = state.dashboard.poll_token.next();
    state.dashboard.poll_token
}

/// `g d`: open the Dashboard view and issue a fresh fetch — "no blank tile
/// waiting for the first tick" (decision 1). Idempotent to press again, and
/// what `r` does too while this view is showing: there is no refresh button
/// anywhere in this app (ADR-0006), only a scoped Refetch, and here the
/// "scope" is the whole server's own vitals rather than one key.
pub(super) fn open_dashboard(mut state: State) -> (State, Vec<Command>) {
    // Leaving Monitor or Pub/Sub closes its feed first, the same rule every
    // other view-switch in this app follows (`open_slowlog`'s own comment).
    let mut commands = leave_slowlog(&mut state);
    commands.extend(leave_monitor(&mut state));
    commands.extend(leave_pubsub(&mut state));
    state.screen = View::Dashboard;
    // On a Cluster the shell reads every node (M5 task 7); the cluster
    // overview is where the view always opens.
    if let Some(c) = state.dashboard.cluster.as_mut() {
        c.undrill();
    }
    state.dashboard.loading = true;
    let token = issue_info_token(&mut state);
    commands.push(Command::FetchServerInfo { token });
    (state, commands)
}

/// `Msg::DashboardPollTick`: the shell's interval always ticks (so it never
/// accrues a missed-tick catch-up burst — `terminal::run`'s own comment),
/// but this is the one place that decides whether a given tick does
/// anything. Three conditions, all of them: the Dashboard is actually on
/// screen (a tick while the reader is on the Slowlog is background work
/// nobody is looking at), the main connection is up (`INFO` against a dead
/// client would just fail), and no poll is already in flight
/// (`state.dashboard.loading` — a slow reply must not get a second request
/// stacked behind it).
pub(super) fn dashboard_poll_tick(mut state: State) -> (State, Vec<Command>) {
    if state.screen != View::Dashboard {
        return (state, Vec::new());
    }
    if !matches!(state.link, crate::state::Link::Up { .. }) {
        return (state, Vec::new());
    }
    if state.dashboard.loading {
        return (state, Vec::new());
    }
    state.dashboard.loading = true;
    let token = issue_info_token(&mut state);
    (state, vec![Command::FetchServerInfo { token }])
}

/// `Msg::ServerInfoLoaded`: record the poll (rotates the previous poll in
/// for the counter-based alarms, pushes ops/sec onto the bounded history,
/// stamps `last_updated_ms` from the shell's injected clock) — but only if
/// `token` names the poll currently outstanding. A stale token (an older
/// manual fetch or poll tick that took longer to answer than a newer one)
/// is dropped whole: applying it would overwrite newer data and — worse —
/// corrupt `record_poll`'s `previous` rotation, which is what the
/// rising-counter alarms compare against (review item (b)).
pub(super) fn server_info_loaded(
    mut state: State,
    info: RawInfo,
    at_ms: u64,
    token: InfoToken,
) -> (State, Vec<Command>) {
    if token != state.dashboard.poll_token {
        return (state, Vec::new());
    }
    state.dashboard.record_poll(info, at_ms);
    (state, Vec::new())
}

/// `Msg::ClusterInfoLoaded`: fold every node's reading in. Each node that
/// newly failed gets its own R7.4 notification naming the command and the node
/// (one per failure, not one per poll); the rest of the Cluster still renders.
pub(super) fn cluster_info_loaded(
    mut state: State,
    nodes: Vec<crate::state::NodeReading>,
    health: Result<String, String>,
    at_ms: u64,
    token: InfoToken,
) -> (State, Vec<Command>) {
    if token != state.dashboard.poll_token {
        return (state, Vec::new());
    }
    let cluster = state.dashboard.cluster.get_or_insert_with(Default::default);
    let failed = cluster.record(nodes, health, at_ms);
    state.dashboard.loading = false;
    state.dashboard.error = None;
    state.dashboard.last_updated_ms = Some(at_ms);
    if let Some((addr, detail)) = failed.first() {
        let more = if failed.len() > 1 {
            format!(" (+{} more)", failed.len() - 1)
        } else {
            String::new()
        };
        state.error = Some((format!("INFO on {addr}: {detail}{more}"), at_ms));
    }
    (state, Vec::new())
}

/// Leaving the Dashboard (any route out): the poll stops with it. On a
/// Cluster that is a command, because the shell holds a connection per node
/// that must not outlive the view; a reply still on its way is dropped by
/// bumping the token, and the next `g d` starts from the cluster overview.
/// A no-op unless the Dashboard is what is showing.
pub(super) fn leave_dashboard(state: &mut State) -> Vec<Command> {
    if state.screen != View::Dashboard {
        return Vec::new();
    }
    if !state.on_cluster() {
        return Vec::new();
    }
    state.dashboard.loading = false;
    state.dashboard.active_mut().close_overlay();
    state.dashboard.poll_token = state.dashboard.poll_token.next();
    if let Some(c) = state.dashboard.cluster.as_mut() {
        c.undrill();
    }
    vec![Command::CloseNodeConnections]
}

/// `Msg::ServerInfoFailed`: the R7.4 notification naming the failing
/// command, and the Dashboard's own in-view error state — the same shape
/// `update::slowlog::slowlog_failed` already has. The last good tiles stay
/// on screen (`DashboardState::raw` is untouched), only their age and an
/// error banner change (decision 7). Guarded by `token` exactly like
/// [`server_info_loaded`].
pub(super) fn server_info_failed(
    mut state: State,
    detail: String,
    at_ms: u64,
    token: InfoToken,
) -> (State, Vec<Command>) {
    if token != state.dashboard.poll_token {
        return (state, Vec::new());
    }
    state.dashboard.loading = false;
    state.dashboard.error = Some(detail.clone());
    state.error = Some((format!("INFO: {detail}"), at_ms));
    (state, Vec::new())
}

/// Actions scoped to the Dashboard view (decision 6, keymap growth rule):
/// tile-focus movement reuses the same four actions Pub/Sub's chip strip
/// already reuses for its own 1-D navigation (`Action::Open`/
/// `Action::CollapseGroup` for horizontal, `Action::MoveUp`/
/// `Action::MoveDown` for vertical — one row of `tiles_per_row` tiles at
/// the current width); `Action::EnterValueCursor` (`Enter`) opens the
/// focused tile's raw `INFO` section as an overlay; `Action::Copy` (`c`)
/// copies it; `Action::Refetch` (`r`) fetches now. While the overlay is
/// open, `MoveUp`/`MoveDown` scroll it instead of moving tile focus —
/// `Action::Cancel` (`Esc`) closing it, rather than leaving the view, is
/// handled in `cancel()` (`update/mod.rs`), the same "nearest thing first"
/// place every other overlay's `Esc` is decided.
pub(super) fn dashboard_dispatch(mut state: State, action: Action) -> (State, Vec<Command>) {
    // On a Cluster the overview is a node table: `j`/`k` move its cursor,
    // `Enter` drills into the node under it. Inside a node view (below) every
    // key is the single-node Dashboard's, acting on that node's own state.
    if state.on_cluster() && !state.dashboard.drilled() {
        return cluster_overview_dispatch(state, action);
    }
    let cols = state.cols;
    if state.dashboard.active().expanded_tile.is_some() {
        return match action {
            Action::MoveUp => {
                let d = state.dashboard.active_mut();
                d.overlay_scroll = d.overlay_scroll.saturating_sub(1);
                (state, Vec::new())
            }
            Action::MoveDown => {
                let d = state.dashboard.active_mut();
                d.overlay_scroll = d.overlay_scroll.saturating_add(1);
                (state, Vec::new())
            }
            Action::Copy => dashboard_copy(state),
            Action::Refetch => dashboard_refetch(state),
            _ => (state, Vec::new()),
        };
    }
    match action {
        Action::MoveUp => {
            state.dashboard.active_mut().focus_up(tiles_per_row(cols));
            (state, Vec::new())
        }
        Action::MoveDown => {
            state.dashboard.active_mut().focus_down(tiles_per_row(cols));
            (state, Vec::new())
        }
        Action::Open => {
            state.dashboard.active_mut().focus_right();
            (state, Vec::new())
        }
        Action::CollapseGroup => {
            state.dashboard.active_mut().focus_left();
            (state, Vec::new())
        }
        Action::EnterValueCursor => {
            state.dashboard.active_mut().open_overlay();
            (state, Vec::new())
        }
        Action::Copy => dashboard_copy(state),
        Action::Refetch => dashboard_refetch(state),
        _ => (state, Vec::new()),
    }
}

/// The Cluster overview's keys (M5 task 7, decision 4): the same actions the
/// single-node grid uses for movement, here moving the node cursor, so no new
/// binding exists. `Enter` drills into the node; `r` polls now.
fn cluster_overview_dispatch(mut state: State, action: Action) -> (State, Vec<Command>) {
    match action {
        Action::MoveUp => {
            if let Some(c) = state.dashboard.cluster.as_mut() {
                c.cursor_up();
            }
            (state, Vec::new())
        }
        Action::MoveDown => {
            if let Some(c) = state.dashboard.cluster.as_mut() {
                c.cursor_down();
            }
            (state, Vec::new())
        }
        Action::EnterValueCursor => {
            if let Some(c) = state.dashboard.cluster.as_mut() {
                c.drill();
            }
            (state, Vec::new())
        }
        Action::Refetch => dashboard_refetch(state),
        _ => (state, Vec::new()),
    }
}

/// `r`, scoped to the Dashboard — the same "never a cache refresh, always a
/// real round trip" rule every other view's `r` follows (ADR-0006).
fn dashboard_refetch(mut state: State) -> (State, Vec<Command>) {
    state.dashboard.loading = true;
    let token = issue_info_token(&mut state);
    (state, vec![Command::FetchServerInfo { token }])
}

/// `c`: copy the focused (or, with the overlay open, the expanded) tile's
/// raw `INFO` section as plain `key:value` text — decision 6's own words,
/// "copies the raw section." A no-op before the first successful poll:
/// there is no raw `INFO` yet to copy a section of.
fn dashboard_copy(state: State) -> (State, Vec<Command>) {
    let node = state.dashboard.active();
    let tile = node.expanded_tile.unwrap_or(node.focused_tile);
    let Some(raw) = node.raw() else {
        return (state, Vec::new());
    };
    let Some(fields) = raw.section(tile.section_name()) else {
        return (state, Vec::new());
    };
    let text = fields
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join("\n");
    (
        state,
        vec![Command::CopyToClipboard {
            text,
            label: format!("{} section", tile.section_name()),
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::KeyCode;
    use crate::state::{RawInfo, TileId};

    fn press(state: State, c: char) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
    }

    fn press_g(state: State, second: char) -> (State, Vec<Command>) {
        let (state, _) = press(state, 'g');
        press(state, second)
    }

    fn loaded(state: State, info: RawInfo, at_ms: u64) -> State {
        let token = state.dashboard.poll_token;
        let (state, _) = update(state, Msg::ServerInfoLoaded { info, at_ms, token });
        state
    }

    #[test]
    fn g_d_opens_the_dashboard_view_and_issues_a_fetch_with_a_fresh_token() {
        let (state, cmds) = press_g(State::default(), 'd');
        assert_eq!(state.screen, View::Dashboard);
        assert!(state.dashboard.loading);
        assert_eq!(
            cmds,
            vec![Command::FetchServerInfo {
                token: state.dashboard.poll_token
            }]
        );
    }

    #[test]
    fn g_k_returns_to_keys_from_the_dashboard() {
        let (state, _) = press_g(State::default(), 'd');
        let (state, _) = press_g(state, 'k');
        assert_eq!(state.screen, View::Keys);
    }

    #[test]
    fn esc_from_the_dashboard_returns_to_keys_with_browser_state_intact() {
        let mut before = State::default();
        before.keys.push(b"k1");
        before.keys.push(b"k2");
        before.rebuild_list();
        before.view.selected = 1;
        let (state, _) = press_g(before.clone(), 'd');
        assert_eq!(state.screen, View::Dashboard);

        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
        assert_eq!(
            state.view.selected, 1,
            "the keys pane's own scroll state survived"
        );
        assert_eq!(state.keys.len(), 2, "the Loaded set was untouched");
    }

    #[test]
    fn opening_the_dashboard_never_touches_tracking_or_the_open_key() {
        let before = State {
            open: Some(crate::state::OpenKey::new(
                Some(0),
                "k".into(),
                crate::state::Value::Str(crate::state::value::StringValue::new("v", 40)),
                -1,
                10,
                0,
            )),
            link: crate::state::Link::Up {
                version: "8.4.0".into(),
                tracking: crate::state::Tracking::Armed,
            },
            ..State::default()
        };
        let (state, _) = press_g(before.clone(), 'd');
        assert_eq!(state.link, before.link, "tracking is untouched");
        assert_eq!(state.open, before.open, "the Open key is untouched");
    }

    #[test]
    fn server_info_loaded_records_the_poll_and_clears_loading() {
        let (state, _) = press_g(State::default(), 'd');
        let state = loaded(
            state,
            RawInfo::new(vec![(
                "Server".to_string(),
                vec![("redis_version".to_string(), "8.4.0".to_string())],
            )]),
            5_000,
        );
        assert!(!state.dashboard.loading);
        assert_eq!(state.dashboard.last_updated_ms, Some(5_000));
        assert!(state.dashboard.error.is_none());
    }

    #[test]
    fn server_info_failed_reports_both_the_toast_and_the_in_view_reason() {
        let (state, _) = press_g(State::default(), 'd');
        let token = state.dashboard.poll_token;
        let (state, _) = update(
            state,
            Msg::ServerInfoFailed {
                detail: "NOPERM this user has no permissions".into(),
                at_ms: 1_000,
                token,
            },
        );
        assert!(!state.dashboard.loading);
        assert_eq!(
            state.dashboard.error.as_deref(),
            Some("NOPERM this user has no permissions")
        );
        let shown = state.error_text().expect("the R7.4 notification");
        assert!(shown.contains("INFO"), "{shown}");
        assert!(shown.contains("NOPERM"), "{shown}");
    }

    #[test]
    fn a_failed_poll_keeps_the_last_good_tiles_on_screen() {
        let (state, _) = press_g(State::default(), 'd');
        let state = loaded(
            state,
            RawInfo::new(vec![(
                "Memory".to_string(),
                vec![("used_memory".to_string(), "123".to_string())],
            )]),
            1_000,
        );
        let before = state.dashboard.raw().cloned();
        // The next poll's token — as if a fresh `r` or poll tick had been
        // issued — then that poll fails.
        let (mut state, _) = update(state, Msg::DashboardPollTick);
        let token = state.dashboard.poll_token;
        (state, _) = update(
            state,
            Msg::ServerInfoFailed {
                detail: "timeout".into(),
                at_ms: 2_000,
                token,
            },
        );
        assert_eq!(
            state.dashboard.raw().cloned(),
            before,
            "the last good raw INFO survives a failed poll"
        );
        assert_eq!(
            state.dashboard.last_updated_ms,
            Some(1_000),
            "the age keeps counting from the last *good* poll, not the failure"
        );
    }

    #[test]
    fn r_refetches_the_dashboard_with_a_new_token() {
        let (state, _) = press_g(State::default(), 'd');
        let state = loaded(state, RawInfo::new(vec![]), 1_000);
        let first_token = state.dashboard.poll_token;
        let (state, cmds) = press(state, 'r');
        assert!(state.dashboard.loading);
        assert_ne!(state.dashboard.poll_token, first_token, "a fresh token");
        assert_eq!(
            cmds,
            vec![Command::FetchServerInfo {
                token: state.dashboard.poll_token
            }]
        );
    }

    // ── item (b): out-of-order replies ──────────────────────────────────────

    #[test]
    fn a_stale_reply_behind_a_newer_manual_fetch_is_dropped() {
        let (state, _) = press_g(State::default(), 'd');
        let stale_token = state.dashboard.poll_token; // the `g d` fetch's own token
        // A manual `r` before the first reply lands — mints a newer token.
        let (state, _) = press(state, 'r');
        let fresh_token = state.dashboard.poll_token;
        assert_ne!(stale_token, fresh_token);

        // The *newer* fetch answers first.
        let (state, _) = update(
            state,
            Msg::ServerInfoLoaded {
                info: RawInfo::new(vec![(
                    "Server".to_string(),
                    vec![("redis_version".to_string(), "8.4.0".to_string())],
                )]),
                at_ms: 2_000,
                token: fresh_token,
            },
        );
        assert_eq!(state.dashboard.last_updated_ms, Some(2_000));

        // The *older*, stale fetch answers second — must be dropped whole,
        // not allowed to overwrite the newer data or roll back the age.
        let (state, _) = update(
            state,
            Msg::ServerInfoLoaded {
                info: RawInfo::new(vec![(
                    "Server".to_string(),
                    vec![("redis_version".to_string(), "7.0.0".to_string())],
                )]),
                at_ms: 1_000,
                token: stale_token,
            },
        );
        assert_eq!(
            state.dashboard.last_updated_ms,
            Some(2_000),
            "the stale reply must not roll back the age"
        );
        assert_eq!(
            state.dashboard.raw().and_then(|r| r.field("redis_version")),
            Some("8.4.0"),
            "the stale reply must not overwrite the newer data"
        );
    }

    #[test]
    fn a_stale_failure_behind_a_newer_success_is_dropped() {
        let (state, _) = press_g(State::default(), 'd');
        let stale_token = state.dashboard.poll_token;
        let (state, _) = press(state, 'r');
        let fresh_token = state.dashboard.poll_token;
        let state = loaded(state, RawInfo::new(vec![]), 1_000);
        assert!(!state.dashboard.loading);
        assert!(state.dashboard.error.is_none());

        let (state, _) = update(
            state,
            Msg::ServerInfoFailed {
                detail: "timeout".into(),
                at_ms: 999,
                token: stale_token,
            },
        );
        assert!(
            state.dashboard.error.is_none(),
            "a stale failure must not clobber a newer success"
        );
        assert!(!state.dashboard.loading);
        let _ = fresh_token;
    }

    #[test]
    fn a_poll_tick_is_skipped_while_off_the_dashboard() {
        let (state, cmds) = update(State::default(), Msg::DashboardPollTick);
        assert_eq!(state.screen, View::Keys);
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_poll_tick_is_skipped_while_disconnected() {
        let (state, _) = press_g(State::default(), 'd');
        let (_, cmds) = update(state, Msg::DashboardPollTick);
        // `State::default()`'s `Link::Connecting` is not `Link::Up`.
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_poll_tick_fires_while_on_the_dashboard_and_connected() {
        let mut state = State::default();
        state.link = crate::state::Link::Up {
            version: "8.4.0".into(),
            tracking: crate::state::Tracking::Unsupported,
        };
        let (state, _) = press_g(state, 'd');
        // `g d` already issued one fetch and left `loading` true — answer it
        // first, so the tick below is testing a *fresh* poll, not a skip
        // caused by `g d`'s own fetch still being in flight.
        let state = loaded(state, RawInfo::new(vec![]), 1_000);
        let before = state.dashboard.poll_token;
        let (state, cmds) = update(state, Msg::DashboardPollTick);
        assert!(state.dashboard.loading);
        assert_ne!(state.dashboard.poll_token, before);
        assert_eq!(
            cmds,
            vec![Command::FetchServerInfo {
                token: state.dashboard.poll_token
            }]
        );
    }

    #[test]
    fn a_poll_tick_is_skipped_while_a_poll_is_already_in_flight() {
        let mut state = State::default();
        state.link = crate::state::Link::Up {
            version: "8.4.0".into(),
            tracking: crate::state::Tracking::Unsupported,
        };
        let (state, _) = press_g(state, 'd'); // loading is already true
        assert!(state.dashboard.loading);
        let before = state.dashboard.poll_token;
        let (state, cmds) = update(state, Msg::DashboardPollTick);
        assert!(cmds.is_empty(), "a poll already in flight is not doubled");
        assert_eq!(state.dashboard.poll_token, before, "no token was minted");
    }

    // ── decision 6: tile-focus movement, the overlay, and `c` ───────────────

    fn connected_dashboard() -> State {
        let mut state = State::default();
        state.link = crate::state::Link::Up {
            version: "8.4.0".into(),
            tracking: crate::state::Tracking::Unsupported,
        };
        let (state, _) = press_g(state, 'd');
        loaded(
            state,
            RawInfo::new(vec![(
                "Replication".to_string(),
                vec![("role".to_string(), "master".to_string())],
            )]),
            1_000,
        )
    }

    #[test]
    fn arrows_and_hjkl_both_move_tile_focus() {
        let state = connected_dashboard();
        assert_eq!(state.dashboard.focused_tile, TileId::Memory);
        let (state, _) = press(state, 'l');
        assert_eq!(state.dashboard.focused_tile, TileId::HitRatio);
        let (state, _) = press(state, 'h');
        assert_eq!(state.dashboard.focused_tile, TileId::Memory);
    }

    #[test]
    fn enter_opens_the_overlay_and_esc_closes_it_without_leaving_the_view() {
        let state = connected_dashboard();
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Enter)));
        assert_eq!(state.dashboard.expanded_tile, Some(TileId::Memory));
        assert_eq!(state.screen, View::Dashboard);

        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(
            state.dashboard.expanded_tile, None,
            "Esc closes the overlay first"
        );
        assert_eq!(
            state.screen,
            View::Dashboard,
            "the nearest-thing-first Esc must not also leave the view"
        );

        // A second Esc, with nothing left to close, does leave the view.
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
    }

    #[test]
    fn c_copies_the_focused_tiles_raw_section() {
        let state = connected_dashboard();
        // Focused tile defaults to Memory, whose section is empty in this
        // fixture (only `Replication` was supplied) — move focus onto
        // Replication instead, which has real content to copy.
        let (state, _) = press(state, 'l'); // HitRatio
        let (state, _) = press(state, 'l'); // Ops
        let (state, _) = press(state, 'l'); // Clients
        let (state, _) = press(state, 'l'); // Replication
        assert_eq!(state.dashboard.focused_tile, TileId::Replication);
        let (_, cmds) = press(state, 'c');
        match cmds.as_slice() {
            [Command::CopyToClipboard { text, label }] => {
                assert!(text.contains("role:master"), "{text}");
                assert!(label.contains("Replication"), "{label}");
            }
            other => panic!("expected one CopyToClipboard command, got {other:?}"),
        }
    }
}

/// M5 task 7: the Dashboard on a Cluster (`docs/plans/m5-dashboard.md`).
#[cfg(test)]
mod cluster_tests {
    use super::*;
    use crate::msg::KeyCode;
    use crate::state::{Environment, NodeReading, NodeRole, Topology};

    fn cluster() -> State {
        let mut state = State::default();
        state.connection.environment = Environment::Staging;
        state.connection.topology = Some(Topology {
            primaries: 3,
            nodes: 6,
        });
        state.link = crate::state::Link::Up {
            version: "8.4.0".into(),
            tracking: crate::state::Tracking::Armed,
        };
        state
    }

    fn key(state: State, code: KeyCode) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(code)))
    }

    fn ch(state: State, c: char) -> (State, Vec<Command>) {
        key(state, KeyCode::Char(c))
    }

    fn info() -> RawInfo {
        RawInfo::new(vec![(
            "Memory".to_string(),
            vec![("used_memory".to_string(), "10".to_string())],
        )])
    }

    fn reading(addr: &str, role: NodeRole, ok: bool) -> NodeReading {
        NodeReading {
            addr: addr.into(),
            role,
            slots: 0,
            info: if ok { Ok(info()) } else { Err("boom".into()) },
        }
    }

    fn health() -> Result<String, String> {
        Ok("cluster_state:ok\r\ncluster_slots_assigned:16384\r\n".into())
    }

    fn open(state: State) -> State {
        let (state, _) = ch(state, 'g');
        ch(state, 'd').0
    }

    fn loaded(state: State, nodes: Vec<NodeReading>) -> State {
        let token = state.dashboard.poll_token;
        update(
            state,
            Msg::ClusterInfoLoaded {
                nodes,
                health: health(),
                at_ms: 7_000,
                token,
            },
        )
        .0
    }

    fn three() -> Vec<NodeReading> {
        vec![
            reading("a:1", NodeRole::Primary, true),
            reading("b:2", NodeRole::Primary, true),
            reading("c:3", NodeRole::Replica, true),
        ]
    }

    #[test]
    fn a_poll_tick_on_a_cluster_fetches_once_and_not_while_one_is_in_flight() {
        let state = open(cluster());
        let (state, commands) = update(state, Msg::DashboardPollTick);
        assert!(commands.is_empty(), "the opening fetch is still in flight");
        let state = loaded(state, three());
        let (state, commands) = update(state, Msg::DashboardPollTick);
        assert!(matches!(
            commands.as_slice(),
            [Command::FetchServerInfo { .. }]
        ));
        assert!(state.dashboard.loading);
    }

    #[test]
    fn a_cluster_reply_records_every_node_and_clears_loading() {
        let state = loaded(open(cluster()), three());
        assert!(!state.dashboard.loading);
        assert_eq!(state.dashboard.last_updated_ms, Some(7_000));
        let c = state.dashboard.cluster.as_ref().unwrap();
        assert_eq!(c.nodes().len(), 3);
    }

    #[test]
    fn a_stale_cluster_reply_is_dropped() {
        let state = open(cluster());
        let stale = state.dashboard.poll_token;
        let (state, _) = ch(state, 'r');
        let (state, _) = update(
            state,
            Msg::ClusterInfoLoaded {
                nodes: three(),
                health: health(),
                at_ms: 1,
                token: stale,
            },
        );
        assert!(state.dashboard.cluster.is_none());
        assert!(state.dashboard.loading, "the newer fetch is still owed");
    }

    #[test]
    fn a_failed_node_raises_one_notification_naming_the_command_and_the_node() {
        let state = loaded(open(cluster()), three());
        let mut nodes = three();
        nodes[1] = reading("b:2", NodeRole::Primary, false);
        let state = loaded(state, nodes.clone());
        let (text, at) = state.error.clone().expect("a notification");
        assert_eq!(text, "INFO on b:2: boom");
        assert_eq!(at, 7_000);
        // The same failure on the next poll does not toast again.
        let mut state = state;
        state.error = None;
        let state = loaded(state, nodes);
        assert!(state.error.is_none());
    }

    #[test]
    fn j_k_move_the_node_cursor_and_enter_drills_in() {
        let state = loaded(open(cluster()), three());
        let (state, _) = ch(state, 'j');
        let (state, commands) = key(state, KeyCode::Enter);
        assert!(commands.is_empty());
        assert!(state.dashboard.drilled());
        let c = state.dashboard.cluster.as_ref().unwrap();
        assert_eq!(c.drilled_node().unwrap().addr, "b:2");
        // The cursor is a node cursor: `k` in the overview goes back up.
    }

    #[test]
    fn inside_a_node_the_single_node_keys_and_raw_info_overlay_work() {
        let state = loaded(open(cluster()), three());
        let (state, _) = key(state, KeyCode::Enter);
        // Tile focus moves in the node's own state.
        let (state, _) = ch(state, 'l');
        assert_eq!(
            state.dashboard.active().focused_tile,
            crate::state::TileId::HitRatio
        );
        let (state, _) = ch(state, 'h');
        // Enter expands the focused tile's raw INFO section.
        let (state, _) = key(state, KeyCode::Enter);
        assert_eq!(
            state.dashboard.active().expanded_tile,
            Some(crate::state::TileId::Memory)
        );
        // `c` copies that section, from the node's own reading.
        let (_, commands) = ch(state.clone(), 'c');
        assert!(matches!(
            commands.as_slice(),
            [Command::CopyToClipboard { text, .. }] if text == "used_memory:10"
        ));
        // Esc closes the overlay, then returns to the overview, then leaves.
        let (state, _) = key(state, KeyCode::Esc);
        assert!(state.dashboard.drilled());
        assert!(state.dashboard.active().expanded_tile.is_none());
        let (state, commands) = key(state, KeyCode::Esc);
        assert!(!state.dashboard.drilled());
        assert_eq!(state.screen, View::Dashboard);
        assert!(commands.is_empty(), "the poll goes on in the overview");
        let (state, commands) = key(state, KeyCode::Esc);
        assert_eq!(state.screen, View::Keys);
        assert_eq!(commands, vec![Command::CloseNodeConnections]);
    }

    #[test]
    fn leaving_the_view_by_any_route_cancels_the_poll_and_drops_a_late_reply() {
        for leave in ['k', 's', 'p'] {
            let state = open(cluster());
            let token = state.dashboard.poll_token;
            let (state, _) = ch(state, 'g');
            let (state, commands) = ch(state, leave);
            assert!(
                commands.contains(&Command::CloseNodeConnections),
                "g {leave}: {commands:?}"
            );
            assert!(!state.dashboard.loading);
            let (state, _) = update(
                state,
                Msg::ClusterInfoLoaded {
                    nodes: three(),
                    health: health(),
                    at_ms: 1,
                    token,
                },
            );
            assert!(state.dashboard.cluster.is_none(), "g {leave}: late reply");
        }
    }

    #[test]
    fn a_poll_tick_after_leaving_issues_nothing() {
        let state = open(cluster());
        let (state, _) = key(state, KeyCode::Esc);
        let (_, commands) = update(state, Msg::DashboardPollTick);
        assert!(commands.is_empty());
    }

    #[test]
    fn a_single_node_dashboard_leaves_without_a_cancel() {
        let (state, _) = update(
            State::default(),
            Msg::Key(KeyPress::plain(KeyCode::Char('g'))),
        );
        let (state, _) = ch(state, 'd');
        let (state, commands) = key(state, KeyCode::Esc);
        assert_eq!(state.screen, View::Keys);
        assert!(commands.is_empty());
    }

    #[test]
    fn reopening_starts_on_the_overview() {
        let state = loaded(open(cluster()), three());
        let (state, _) = key(state, KeyCode::Enter);
        let (state, _) = ch(state, 'g');
        let (state, _) = ch(state, 'k');
        let state = open(state);
        assert!(!state.dashboard.drilled());
    }

    #[test]
    fn a_whole_poll_failure_keeps_the_last_good_nodes() {
        let state = loaded(open(cluster()), three());
        let (state, _) = ch(state, 'r');
        let token = state.dashboard.poll_token;
        let (state, _) = update(
            state,
            Msg::ServerInfoFailed {
                detail: "CLUSTER NODES failed on every node".into(),
                at_ms: 9_000,
                token,
            },
        );
        assert!(state.dashboard.error.is_some());
        assert_eq!(state.dashboard.cluster.as_ref().unwrap().nodes().len(), 3);
    }
}
