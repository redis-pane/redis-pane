//! The Slowlog view (`g s`, R6.4, M3, `docs/plans/m3-slowlog.md`): opening
//! it, its own movement/sort/refetch/copy, and the two replies
//! `Command::FetchSlowlog` can answer with.
//!
//! Switching into or out of this view touches exactly `State::screen` and,
//! on the way in, `State::slowlog` — never `State::open`, `State::link`, or
//! anything a Refetch/tracking arming depends on, so opening the Slowlog
//! view and coming back never disarms tracking on the Open key (M3 phase A's
//! own invariant).

use super::*;
use crate::state::{SlowlogEntry, View};

/// How many entries `SLOWLOG GET` is asked for. `slowlog-max-len` defaults to
/// 128; comfortably above that so a fetch is normally the server's whole
/// ring buffer, not a truncated prefix of it.
pub(super) const SLOWLOG_FETCH_COUNT: i64 = 256;

/// How many rows one `PageUp`/`PageDown` moves the selection — fixed rather
/// than `State::visible_rows()` (the keys pane's own figure), since that
/// reads geometry this view does not share (ADR-0011: no geometry in the
/// core beyond `cols`/`rows` themselves, and this is smaller than a whole
/// pane's height anyway).
pub(super) const SLOWLOG_PAGE_ROWS: usize = 10;

/// `g s`: open the Slowlog view and issue a fresh fetch. Idempotent to press
/// again, and what `r` does too while this view is showing — there is no
/// refresh button anywhere in this app (ADR-0006), only a scoped Refetch,
/// and here the "scope" is the whole ring buffer rather than one key.
pub(super) fn open_slowlog(mut state: State) -> (State, Vec<Command>) {
    // Leaving Monitor or Pub/Sub closes its feed first (decision 2,
    // `docs/plans/m3-monitor.md`; `docs/plans/m3-pubsub.md`) — a no-op
    // unless one was actually open.
    let mut commands = leave_monitor(&mut state);
    commands.extend(leave_pubsub(&mut state));
    state.screen = View::Slowlog;
    // On a Cluster `SLOWLOG GET` would answer from an arbitrary node and show
    // it as the whole Cluster's (ADR-0022); the view renders a notice instead
    // and nothing is fetched until M5 task 8.
    if state.on_cluster() {
        state.slowlog.loading = false;
        return (state, commands);
    }
    state.slowlog.loading = true;
    commands.push(Command::FetchSlowlog {
        count: SLOWLOG_FETCH_COUNT,
    });
    (state, commands)
}

/// `g k`: back to the Keys view. A no-op if already there — nothing here
/// depends on which view was showing, except a Monitor or Pub/Sub feed left
/// open (closed here too).
pub(super) fn open_keys_view(mut state: State) -> (State, Vec<Command>) {
    let mut commands = leave_monitor(&mut state);
    commands.extend(leave_pubsub(&mut state));
    state.screen = View::Keys;
    (state, commands)
}

/// `Msg::SlowlogLoaded`.
pub(super) fn slowlog_loaded(
    mut state: State,
    entries: Vec<SlowlogEntry>,
) -> (State, Vec<Command>) {
    state.slowlog.loading = false;
    state.slowlog.error = None;
    state.slowlog.set_entries(entries);
    (state, Vec::new())
}

/// `Msg::SlowlogFailed`: the R7.4 notification naming the failing command,
/// and the in-view empty state's own words — the same fact, read by the
/// global toast and by the screen it happened on.
pub(super) fn slowlog_failed(
    mut state: State,
    detail: String,
    at_ms: u64,
) -> (State, Vec<Command>) {
    state.slowlog.loading = false;
    state.slowlog.error = Some(detail.clone());
    state.error = Some((format!("SLOWLOG GET: {detail}"), at_ms));
    (state, Vec::new())
}

/// `Mutation::ResetSlowlog` settled `Done`: a notice, then the same refetch
/// every other write's success goes through (ADR-0006) — applied to the
/// whole ring buffer rather than one key, since `RESET` has no key at all
/// (D8, `docs/plans/m3-slowlog.md`). Reuses `open_slowlog` for the refetch
/// half rather than a second `Command::FetchSlowlog` site.
pub(super) fn reset_slowlog_landed(mut state: State, at_ms: u64) -> (State, Vec<Command>) {
    state.notice = Some(("slowlog reset".to_string(), at_ms));
    open_slowlog(state)
}

/// Movement, sort, refetch, copy and reset, scoped to the Slowlog view (PLAN
/// decision 3) — `dispatch_action`'s own guard routes here for exactly the
/// actions this view gives a meaning to; everything else is a no-op while
/// it is showing.
pub(super) fn slowlog_dispatch(mut state: State, action: Action) -> (State, Vec<Command>) {
    // Nothing here has a meaning on a Cluster (`open_slowlog`): not `r`, and
    // above all not `d`, which would stage `SLOWLOG RESET` against whichever
    // node answered.
    if state.on_cluster() {
        return (state, Vec::new());
    }
    match action {
        Action::MoveDown => {
            state.slowlog.move_selection(1);
            (state, Vec::new())
        }
        Action::MoveUp => {
            state.slowlog.move_selection(-1);
            (state, Vec::new())
        }
        Action::PageDown => {
            state.slowlog.move_selection(SLOWLOG_PAGE_ROWS as isize);
            (state, Vec::new())
        }
        Action::PageUp => {
            state.slowlog.move_selection(-(SLOWLOG_PAGE_ROWS as isize));
            (state, Vec::new())
        }
        Action::Top => {
            state.slowlog.top();
            (state, Vec::new())
        }
        Action::Bottom => {
            state.slowlog.bottom();
            (state, Vec::new())
        }
        Action::Sort => {
            state.slowlog.cycle_sort();
            (state, Vec::new())
        }
        // A plain re-fetch, matching what `r` means everywhere else in this
        // app: never a cache refresh, always a real round trip (ADR-0006).
        Action::Refetch => open_slowlog(state),
        Action::Copy => slowlog_copy(state),
        // `d`: stage `SLOWLOG RESET` (M3 phase B, Decision 3 — reuses
        // `Action::Delete`, the same one-Action-two-verbs shape `t` already
        // has for tree/ttl, dispatched by view). Composed unconditionally,
        // regardless of Read-only Mode — the chokepoint refuses at `y`, not
        // here (R4.4, the same rule every other staged mutation follows).
        Action::Delete => {
            state.confirm = Some(crate::state::PendingMutation::ResetSlowlog);
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// `c`: copy the selected entry's command text to the clipboard — the one
/// thing worth copying here, the same "no mnemonic, no chord" simplicity
/// `Action::Copy` has everywhere else (DESIGN §4). Uses the Viewer's own
/// byte-escaping (`cell_text`) so a binary argument copies the same way a
/// binary collection member would.
fn slowlog_copy(state: State) -> (State, Vec<Command>) {
    let Some(entry) = state.slowlog.selected_entry() else {
        return (state, Vec::new());
    };
    let text = crate::state::value::cell_text(&entry.command);
    (
        state,
        vec![Command::CopyToClipboard {
            text,
            label: "command".into(),
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::KeyCode;
    use crate::state::SlowlogEntry;

    fn entry(id: i64, duration_us: i64) -> SlowlogEntry {
        SlowlogEntry {
            id,
            timestamp: 1_700_000_000,
            duration_us,
            command: format!("GET k{id}").into_bytes(),
            client_addr: b"127.0.0.1:1234".to_vec(),
            client_name: Vec::new(),
        }
    }

    fn press(state: State, c: char) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
    }

    fn press_g(state: State, second: char) -> (State, Vec<Command>) {
        let (state, _) = press(state, 'g');
        press(state, second)
    }

    #[test]
    fn g_s_opens_the_slowlog_view_and_issues_a_fetch() {
        let (state, cmds) = press_g(State::default(), 's');
        assert_eq!(state.screen, View::Slowlog);
        assert!(state.slowlog.loading);
        assert_eq!(
            cmds,
            vec![Command::FetchSlowlog {
                count: SLOWLOG_FETCH_COUNT
            }]
        );
    }

    #[test]
    fn g_by_itself_does_nothing_and_arms_the_pending_chord() {
        let (state, cmds) = press(State::default(), 'g');
        assert_eq!(
            state.pending_chord,
            Some(KeyPress::plain(KeyCode::Char('g')))
        );
        assert_eq!(state.screen, View::Keys);
        assert!(cmds.is_empty());
    }

    #[test]
    fn esc_after_g_clears_the_pending_chord_without_acting() {
        let (state, _) = press(State::default(), 'g');
        assert!(state.pending_chord.is_some());
        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert!(state.pending_chord.is_none());
        assert_eq!(state.screen, View::Keys);
        assert!(cmds.is_empty());
    }

    #[test]
    fn an_unbound_second_key_clears_the_pending_chord_and_does_nothing() {
        let (state, cmds) = press_g(State::default(), 'z');
        assert!(state.pending_chord.is_none());
        assert_eq!(state.screen, View::Keys);
        assert!(cmds.is_empty());
    }

    #[test]
    fn esc_from_the_slowlog_view_returns_to_keys_with_browser_state_intact() {
        let mut before = State::default();
        before.keys.push(b"k1");
        before.keys.push(b"k2");
        before.rebuild_list();
        before.view.selected = 1;
        let (state, _) = press_g(before.clone(), 's');
        assert_eq!(state.screen, View::Slowlog);

        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
        assert_eq!(
            state.view.selected, 1,
            "the keys pane's own scroll state survived"
        );
        assert_eq!(state.keys.len(), 2, "the Loaded set was untouched");
    }

    #[test]
    fn g_k_returns_to_keys_from_anywhere() {
        let (state, _) = press_g(State::default(), 's');
        let (state, _) = press_g(state, 'k');
        assert_eq!(state.screen, View::Keys);
    }

    #[test]
    fn opening_the_slowlog_view_never_touches_tracking_or_the_open_key() {
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
        let (state, _) = press_g(before.clone(), 's');
        assert_eq!(state.link, before.link, "tracking is untouched");
        assert_eq!(state.open, before.open, "the Open key is untouched");
    }

    #[test]
    fn slowlog_loaded_fills_entries_and_clears_loading() {
        let (state, _) = press_g(State::default(), 's');
        let (state, _) = update(
            state,
            Msg::SlowlogLoaded {
                entries: vec![entry(1, 100), entry(2, 500)],
            },
        );
        assert!(!state.slowlog.loading);
        assert_eq!(state.slowlog.len(), 2);
        assert!(state.slowlog.error.is_none());
    }

    #[test]
    fn slowlog_failed_reports_both_the_toast_and_the_in_view_reason() {
        let (state, _) = press_g(State::default(), 's');
        let (state, _) = update(
            state,
            Msg::SlowlogFailed {
                detail: "ERR unknown command".into(),
                at_ms: 1_000,
            },
        );
        assert!(!state.slowlog.loading);
        assert_eq!(state.slowlog.error.as_deref(), Some("ERR unknown command"));
        let shown = state.error_text().expect("the R7.4 notification");
        assert!(shown.contains("SLOWLOG GET"), "{shown}");
        assert!(shown.contains("ERR unknown command"), "{shown}");
    }

    #[test]
    fn sort_permutes_the_index_via_the_view_scoped_s_key() {
        let (mut state, _) = press_g(State::default(), 's');
        state.slowlog.set_entries(vec![entry(1, 50), entry(2, 500)]);
        assert_eq!(state.slowlog.sort, crate::state::SlowlogSort::Recent);
        let (state, cmds) = press(state, 's');
        assert_eq!(state.slowlog.sort, crate::state::SlowlogSort::Duration);
        assert!(cmds.is_empty());
    }

    #[test]
    fn r_refetches_the_whole_slowlog() {
        let (state, _) = press_g(State::default(), 's');
        let (state, cmds) = press(state, 'r');
        assert!(state.slowlog.loading);
        assert_eq!(
            cmds,
            vec![Command::FetchSlowlog {
                count: SLOWLOG_FETCH_COUNT
            }]
        );
    }

    #[test]
    fn d_stages_reset_slowlog_rather_than_executing_immediately() {
        let (state, _) = press_g(State::default(), 's');
        let mut state = state;
        state.slowlog.set_entries(vec![entry(1, 50)]);
        let (state, cmds) = press(state, 'd');
        assert_eq!(
            state.confirm,
            Some(crate::state::PendingMutation::ResetSlowlog)
        );
        assert!(cmds.is_empty(), "nothing runs before it is confirmed");
    }

    #[test]
    fn confirming_a_staged_reset_issues_slowlog_reset() {
        let (state, _) = press_g(State::default(), 's');
        let mut state = state;
        state.slowlog.set_entries(vec![entry(1, 50)]);
        let (state, _) = press(state, 'd');
        let (state, cmds) = press(state, 'y');
        assert!(state.confirm.is_none(), "the dialog closes on confirm");
        assert_eq!(
            cmds,
            vec![Command::Execute {
                mutation: Mutation::ResetSlowlog,
                index: None,
            }]
        );
    }

    #[test]
    fn read_only_refuses_reset_slowlog_at_confirm_for_every_reason_including_replica() {
        for reason in [
            crate::state::ReadOnlyReason::Environment,
            crate::state::ReadOnlyReason::User,
            crate::state::ReadOnlyReason::Replica,
        ] {
            let (mut state, _) = press_g(State::default(), 's');
            state.slowlog.set_entries(vec![entry(1, 50)]);
            state.read_only = Some(reason);
            let (state, _) = press(state, 'd');
            assert!(state.confirm.is_some(), "the preview is composed anyway");
            let (state, cmds) = press(state, 'y');
            assert!(state.confirm.is_none(), "the dialog still closes");
            assert!(
                !cmds.iter().any(|c| matches!(
                    c,
                    Command::Execute {
                        mutation: Mutation::ResetSlowlog,
                        ..
                    }
                )),
                "{reason:?}: nothing was actually sent to the server"
            );
        }
    }

    #[test]
    fn a_settled_reset_notices_and_refetches_the_whole_slowlog() {
        let (state, _) = press_g(State::default(), 's');
        let (state, cmds) = update(
            state,
            Msg::MutationSettled {
                mutation: Mutation::ResetSlowlog,
                index: None,
                result: Ok(crate::mutation::MutationOutcome::Done),
                at_ms: 1_000,
            },
        );
        assert_eq!(
            state.notice.as_ref().map(|(t, _)| t.as_str()),
            Some("slowlog reset")
        );
        assert!(state.slowlog.loading);
        assert_eq!(
            cmds,
            vec![Command::FetchSlowlog {
                count: SLOWLOG_FETCH_COUNT
            }]
        );
    }

    #[test]
    fn movement_and_copy_are_scoped_to_the_slowlog_view() {
        let (mut state, _) = press_g(State::default(), 's');
        state.slowlog.set_entries(vec![entry(1, 50), entry(2, 500)]);
        let (state, _) = press(state, 'j');
        assert_eq!(state.slowlog.selected, 1);
        let (_, cmds) = press(state, 'c');
        assert!(matches!(cmds.as_slice(), [Command::CopyToClipboard { .. }]));
    }
}
