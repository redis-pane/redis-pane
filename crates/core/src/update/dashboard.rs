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
    let mut commands = leave_monitor(&mut state);
    commands.extend(leave_pubsub(&mut state));
    state.screen = View::Dashboard;
    // On a Cluster `INFO` would answer from an arbitrary node and show its
    // figures as the whole Cluster's (ADR-0022): a notice, and no fetch,
    // until M5 task 8.
    if state.on_cluster() {
        state.dashboard.loading = false;
        return (state, commands);
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
    if state.screen != View::Dashboard || state.on_cluster() {
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
    // No tiles, no overlay and no fetch on a Cluster (`open_dashboard`).
    if state.on_cluster() {
        return (state, Vec::new());
    }
    if state.dashboard.expanded_tile.is_some() {
        return match action {
            Action::MoveUp => {
                state.dashboard.overlay_scroll = state.dashboard.overlay_scroll.saturating_sub(1);
                (state, Vec::new())
            }
            Action::MoveDown => {
                state.dashboard.overlay_scroll = state.dashboard.overlay_scroll.saturating_add(1);
                (state, Vec::new())
            }
            Action::Copy => dashboard_copy(state),
            Action::Refetch => dashboard_refetch(state),
            _ => (state, Vec::new()),
        };
    }
    match action {
        Action::MoveUp => {
            state.dashboard.focus_up(tiles_per_row(state.cols));
            (state, Vec::new())
        }
        Action::MoveDown => {
            state.dashboard.focus_down(tiles_per_row(state.cols));
            (state, Vec::new())
        }
        Action::Open => {
            state.dashboard.focus_right();
            (state, Vec::new())
        }
        Action::CollapseGroup => {
            state.dashboard.focus_left();
            (state, Vec::new())
        }
        Action::EnterValueCursor => {
            state.dashboard.open_overlay();
            (state, Vec::new())
        }
        Action::Copy => dashboard_copy(state),
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
    let tile = state
        .dashboard
        .expanded_tile
        .unwrap_or(state.dashboard.focused_tile);
    let Some(raw) = state.dashboard.raw() else {
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
