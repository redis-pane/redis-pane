//! The Monitor view (`g m`, R6.1, M3 phase B, `docs/plans/m3-monitor.md`):
//! opening it (with the `prod`/`unknown` cost confirmation), closing the feed
//! on every way out, movement/pause/filter/copy/reopen, and folding an
//! arriving `Msg::MonitorLine` into the bounded tail.
//!
//! Switching into this view touches `State::screen` and `State::monitor`,
//! same as `update::slowlog` touches `State::screen`/`State::slowlog` — never
//! `State::open`/`State::link`, so a round trip through Monitor never disarms
//! tracking on the Open key, the same invariant the Slowlog view already
//! holds.

use super::*;
use crate::command::FeedKindMsg;
use crate::state::{FeedStatus, View};

/// How many lines one `PageUp`/`PageDown` moves the selection — the same
/// fixed figure `update::slowlog::SLOWLOG_PAGE_ROWS` uses, and for the same
/// reason: this view has no `State::visible_rows()` geometry of its own to
/// read (ADR-0011).
pub(super) const MONITOR_PAGE_ROWS: usize = 10;

/// `g m`: open the Monitor view. `prod`/`unknown` stage the cost-confirmation
/// dialog instead of opening directly (decision 3); `local`/`staging` open
/// straight away. Reachable from any screen, including from inside the
/// Monitor view itself — pressing `g m` again always restarts fresh
/// (decision 2), same as every other entry.
pub(super) fn open_monitor(mut state: State) -> (State, Vec<Command>) {
    match state.connection.environment {
        crate::state::Environment::Prod | crate::state::Environment::Unknown => {
            state.pending_feed = Some(FeedKindMsg::Monitor);
            (state, Vec::new())
        }
        crate::state::Environment::Local | crate::state::Environment::Staging => {
            open_monitor_view(state)
        }
    }
}

/// Actually open the view: close whatever feed was already open (decision 2
/// — every `g m` starts fresh, even a `g m` pressed again from inside the
/// view), reset the tail, mint a token, and ask the shell to dial.
pub(super) fn open_monitor_view(mut state: State) -> (State, Vec<Command>) {
    let mut commands = leave_monitor(&mut state);
    commands.extend(leave_pubsub(&mut state));
    state.screen = View::Monitor;
    state.monitor.reset();
    state.monitor.feed_token = issue_feed_token(&mut state);
    state.monitor.status = FeedStatus::Connecting;
    commands.push(Command::OpenFeed {
        kind: FeedKindMsg::Monitor,
        token: state.monitor.feed_token,
    });
    (state, commands)
}

/// Close whatever feed the Monitor view has open, if any. The one step every
/// route out of the view takes — `Esc`, `g k`, `g s`, quit (decision 2) — and
/// the first step [`open_monitor_view`] itself takes before opening a fresh
/// one. A no-op unless the feed is actually `Connecting`/`Open`: nothing to
/// close from `Idle` or an already-`Closed` feed, so leaving-then-leaving-
/// again (or leaving a screen that was never Monitor at all) never emits a
/// spurious `Command::CloseFeed`.
pub(super) fn leave_monitor(state: &mut State) -> Vec<Command> {
    if matches!(
        state.monitor.status,
        FeedStatus::Connecting | FeedStatus::Open
    ) {
        state.monitor.status = FeedStatus::Idle;
        vec![Command::CloseFeed]
    } else {
        Vec::new()
    }
}

/// `r` while the Monitor view is showing a closed (or never-opened) feed:
/// reopen it *without* resetting the tail (decision 9: "the view keeps the
/// buffer on screen ... `r` reopens") — the one way `r` differs from `g m`,
/// which always starts fresh (decision 2). A no-op while the feed is already
/// `Connecting`/`Open` — there is nothing to reopen, and no refresh button
/// anywhere in this app (ADR-0006) that `r` on a live feed could mean.
pub(super) fn reopen_monitor_feed(mut state: State) -> (State, Vec<Command>) {
    if matches!(
        state.monitor.status,
        FeedStatus::Connecting | FeedStatus::Open
    ) {
        return (state, Vec::new());
    }
    state.monitor.feed_token = issue_feed_token(&mut state);
    state.monitor.status = FeedStatus::Connecting;
    let token = state.monitor.feed_token;
    (
        state,
        vec![Command::OpenFeed {
            kind: FeedKindMsg::Monitor,
            token,
        }],
    )
}

/// `y`/`Esc` on the `MONITOR` cost-confirmation dialog (decision 3) — the
/// sibling of `update::confirm::confirm_key` for a pending kind that is
/// deliberately *not* a [`crate::state::PendingMutation`]: opening a view is
/// not a write, so Read-only Mode's chokepoint must never see this, and there
/// is no diff or command preview to show, only the one line naming
/// `MONITOR`'s cost (`crate::render::feed_confirm_overlay`). Every other key
/// is swallowed and the dialog put back, the same "stray keystroke never
/// silently discards what's staged" rule `confirm_key` follows.
pub(super) fn feed_confirm_key(
    mut state: State,
    kind: FeedKindMsg,
    key: KeyPress,
) -> (State, Vec<Command>) {
    match key.code {
        KeyCode::Char('y') if !key.ctrl && !key.alt => match kind {
            FeedKindMsg::Monitor => open_monitor_view(state),
            // Pub/Sub stages no confirmation (`m3-pubsub.md` decision 7:
            // "no banner and no prod confirm" — subscribing is scoped to
            // what the reader chose, not a server-wide cost). Nothing ever
            // sets `state.pending_feed = Some(FeedKindMsg::Subscribe(_))`
            // today; this arm only keeps the match exhaustive ahead of
            // phase B, which opens `View::PubSub` directly instead.
            FeedKindMsg::Subscribe(_) => (state, Vec::new()),
        },
        KeyCode::Esc => (state, Vec::new()),
        _ => {
            state.pending_feed = Some(kind);
            (state, Vec::new())
        }
    }
}

/// Movement, pause, reopen, copy and filter, scoped to the Monitor view —
/// `dispatch_action`'s own guard routes here for exactly the actions this
/// view gives a meaning to; everything else is a no-op while it is showing.
pub(super) fn monitor_dispatch(mut state: State, action: Action) -> (State, Vec<Command>) {
    match action {
        Action::MoveDown => {
            state.monitor.move_selection(1);
            (state, Vec::new())
        }
        Action::MoveUp => {
            state.monitor.move_selection(-1);
            (state, Vec::new())
        }
        Action::PageDown => {
            state.monitor.move_selection(MONITOR_PAGE_ROWS as isize);
            (state, Vec::new())
        }
        Action::PageUp => {
            state.monitor.move_selection(-(MONITOR_PAGE_ROWS as isize));
            (state, Vec::new())
        }
        Action::Top => {
            state.monitor.to_top();
            (state, Vec::new())
        }
        // `End`'s bare binding is `Action::Bottom` (`Keymap::default`) — this
        // is decision 7's "resume following" affordance, reached the same
        // way the keys pane's own `End` reaches the bottom of the list.
        Action::Bottom => {
            state.monitor.to_bottom();
            (state, Vec::new())
        }
        // Pause/resume means nothing with no feed to pause: gated on `Open`
        // so a stray `p` while connecting or after a close does not flip a
        // flag the header would then have to explain away as meaningless
        // (the closed/connecting states offer `r reopen` instead — see
        // `crate::help::monitor_rows`, which hides this row exactly when
        // this guard would make it a no-op).
        Action::TogglePause if state.monitor.status == FeedStatus::Open => {
            state.monitor.toggle_pause();
            (state, Vec::new())
        }
        Action::TogglePause => (state, Vec::new()),
        Action::Refetch => reopen_monitor_feed(state),
        Action::Copy => monitor_copy(state),
        Action::Filter => {
            state.filtering = true;
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// `c`: copy the selected line's command — the quoted command-and-args
/// portion `MonitorLine::columns` parses out, run through the Viewer's own
/// byte-escaping (decision 8), the same `cell_text` the Slowlog view's own
/// copy already uses.
fn monitor_copy(state: State) -> (State, Vec<Command>) {
    let Some(line) = state.monitor.selected_line() else {
        return (state, Vec::new());
    };
    let text = crate::state::value::cell_text(line.columns().command.as_bytes());
    (
        state,
        vec![Command::CopyToClipboard {
            text,
            label: "command".into(),
        }],
    )
}

/// Keys typed while the Monitor view's filter is capturing (`/`) — the same
/// modal capture UX the keys pane's own filter uses (`update::keys::filter_key`,
/// `state.filtering`), but the pattern narrows `MonitorState::filter` for
/// display only (decision 6): there is no `rebuild` step, because filtering
/// here is a render-time concern over the buffer already held, not a second
/// index to keep in step with the arena.
pub(super) fn monitor_filter_key(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    match key.code {
        KeyCode::Esc => {
            state.filtering = false;
            state.monitor.filter.clear();
            (state, Vec::new())
        }
        KeyCode::Enter => {
            state.filtering = false;
            (state, Vec::new())
        }
        KeyCode::Backspace => {
            state.monitor.filter.pop();
            (state, Vec::new())
        }
        KeyCode::Char(c) if !key.ctrl && !key.alt => {
            state.monitor.filter.push(c);
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// `Msg::MonitorLine`: fold a line from the feed into the bounded tail.
///
/// Guarded twice: `token` must still name the feed currently open (the same
/// discipline `Msg::FeedOpened`/`Msg::FeedClosed` follow — a line from a feed
/// the reader has since left, or superseded with another `g m`, is stale),
/// and the Monitor view must still be showing — closing the feed on the way
/// out (`leave_monitor`) does not itself change the token, so a handful of
/// lines already in flight when `Command::CloseFeed` was issued could still
/// arrive with a token that matches; this second check is what stops them
/// from growing a buffer nobody is looking at.
pub(super) fn monitor_line(
    mut state: State,
    token: crate::command::FeedToken,
    at_ms: u64,
    raw: String,
) -> (State, Vec<Command>) {
    if token == state.monitor.feed_token && state.screen == View::Monitor {
        state.monitor.push_monitor_line(at_ms, raw);
    }
    (state, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::KeyCode;
    use crate::state::{Connection, Environment};

    fn press(state: State, c: char) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
    }

    fn press_g(state: State, second: char) -> (State, Vec<Command>) {
        let (state, _) = press(state, 'g');
        press(state, second)
    }

    fn with_env(env: Environment) -> State {
        State {
            connection: Connection {
                environment: env,
                ..Connection::default()
            },
            ..State::default()
        }
    }

    #[test]
    fn g_m_opens_directly_in_local_and_staging() {
        for env in [Environment::Local, Environment::Staging] {
            let (state, cmds) = press_g(with_env(env), 'm');
            assert_eq!(state.screen, View::Monitor, "{env:?}");
            assert!(state.pending_feed.is_none(), "{env:?}");
            assert!(
                matches!(
                    cmds.as_slice(),
                    [Command::OpenFeed {
                        kind: FeedKindMsg::Monitor,
                        ..
                    }]
                ),
                "{env:?}: {cmds:?}"
            );
        }
    }

    #[test]
    fn g_m_stages_a_confirmation_in_prod_and_unknown_rather_than_opening() {
        for env in [Environment::Prod, Environment::Unknown] {
            let (state, cmds) = press_g(with_env(env), 'm');
            assert_eq!(state.screen, View::Keys, "{env:?}: not opened yet");
            assert_eq!(state.pending_feed, Some(FeedKindMsg::Monitor), "{env:?}");
            assert!(cmds.is_empty(), "{env:?}: nothing dials until confirmed");
        }
    }

    #[test]
    fn y_on_the_prod_confirmation_opens_the_view() {
        let (state, _) = press_g(with_env(Environment::Prod), 'm');
        let (state, cmds) = press(state, 'y');
        assert_eq!(state.screen, View::Monitor);
        assert!(state.pending_feed.is_none());
        assert!(matches!(cmds.as_slice(), [Command::OpenFeed { .. }]));
    }

    #[test]
    fn esc_on_the_prod_confirmation_opens_nothing() {
        let (state, _) = press_g(with_env(Environment::Prod), 'm');
        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
        assert!(state.pending_feed.is_none());
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_stray_key_on_the_confirmation_is_swallowed_not_discarded() {
        let (state, _) = press_g(with_env(Environment::Prod), 'm');
        let (state, cmds) = press(state, 'z');
        assert_eq!(
            state.pending_feed,
            Some(FeedKindMsg::Monitor),
            "the dialog is still staged"
        );
        assert!(cmds.is_empty());
    }

    #[test]
    fn read_only_does_not_refuse_opening_monitor() {
        // Read-only Mode must never see this confirmation at all — opening a
        // view is not a write (decision 3).
        let mut state = with_env(Environment::Prod);
        state.read_only = Some(crate::state::ReadOnlyReason::Environment);
        let (state, _) = press_g(state, 'm');
        let (state, cmds) = press(state, 'y');
        assert_eq!(state.screen, View::Monitor);
        assert!(matches!(cmds.as_slice(), [Command::OpenFeed { .. }]));
    }

    #[test]
    fn esc_from_the_monitor_view_closes_the_feed_and_returns_to_keys() {
        let (state, _) = press_g(State::default(), 'm');
        assert_eq!(state.screen, View::Monitor);
        let token = state.monitor.feed_token;
        let (state, _) = update(state, Msg::FeedOpened { token });
        assert_eq!(state.monitor.status, FeedStatus::Open);

        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
        assert_eq!(state.monitor.status, FeedStatus::Idle);
        assert_eq!(cmds, vec![Command::CloseFeed]);
    }

    #[test]
    fn g_k_from_monitor_closes_the_feed() {
        let (state, _) = press_g(State::default(), 'm');
        let (state, cmds) = press_g(state, 'k');
        assert_eq!(state.screen, View::Keys);
        assert_eq!(cmds, vec![Command::CloseFeed]);
    }

    #[test]
    fn g_s_from_monitor_closes_the_feed_and_opens_slowlog() {
        let (state, _) = press_g(State::default(), 'm');
        let (state, cmds) = press_g(state, 's');
        assert_eq!(state.screen, View::Slowlog);
        assert!(cmds.contains(&Command::CloseFeed));
    }

    #[test]
    fn quitting_from_monitor_closes_the_feed_too() {
        let (state, _) = press_g(State::default(), 'm');
        let (state, cmds) = press(state, 'q');
        assert!(state.quitting);
        assert!(cmds.contains(&Command::CloseFeed));
        assert!(cmds.contains(&Command::Quit));
    }

    #[test]
    fn leaving_a_view_that_was_never_monitor_emits_no_close_feed() {
        let (state, _) = press_g(State::default(), 's');
        let (state, cmds) = press_g(state, 'k');
        assert_eq!(state.screen, View::Keys);
        assert!(cmds.is_empty());
    }

    #[test]
    fn g_m_again_from_inside_the_view_closes_the_old_feed_and_starts_fresh() {
        let (state, _) = press_g(State::default(), 'm');
        let old_token = state.monitor.feed_token;
        let (mut state, _) = update(state, Msg::FeedOpened { token: old_token });
        state.monitor.push_monitor_line(1, "old line".into());
        assert_eq!(state.monitor.len(), 1);

        let (state, cmds) = press_g(state, 'm');
        assert_eq!(state.screen, View::Monitor);
        assert!(state.monitor.is_empty(), "the tail resets");
        assert_ne!(state.monitor.feed_token, old_token);
        assert!(cmds.contains(&Command::CloseFeed));
        assert!(cmds.iter().any(|c| matches!(c, Command::OpenFeed { .. })));
    }

    #[test]
    fn a_monitor_line_folds_into_the_buffer_when_the_token_matches() {
        let (state, _) = press_g(State::default(), 'm');
        let token = state.monitor.feed_token;
        let (state, _) = update(
            state,
            Msg::MonitorLine {
                token,
                at_ms: 1,
                raw: "line one".into(),
            },
        );
        assert_eq!(state.monitor.len(), 1);
    }

    #[test]
    fn a_stale_monitor_line_is_dropped() {
        let (state, _) = press_g(State::default(), 'm');
        let stale = state.monitor.feed_token;
        // Supersede with a fresh `g m`.
        let (state, _) = press_g(state, 'm');
        let (state, _) = update(
            state,
            Msg::MonitorLine {
                token: stale,
                at_ms: 1,
                raw: "stale line".into(),
            },
        );
        assert!(state.monitor.is_empty());
    }

    #[test]
    fn a_monitor_line_after_leaving_the_view_is_dropped_even_with_a_matching_token() {
        let (state, _) = press_g(State::default(), 'm');
        let token = state.monitor.feed_token;
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
        let (state, _) = update(
            state,
            Msg::MonitorLine {
                token,
                at_ms: 1,
                raw: "arrived too late".into(),
            },
        );
        assert!(state.monitor.is_empty());
    }

    #[test]
    fn p_toggles_pause_only_while_the_feed_is_open() {
        let (state, _) = press(State::default(), 'p');
        assert!(!state.monitor.paused, "a no-op outside the Monitor view");

        // Connecting, not yet Open: `p` is still a no-op — pausing a feed
        // that has not started streaming is meaningless, the same reason it
        // is hidden from the hint bar/help in this state
        // (`crate::help::monitor_rows`).
        let (state, _) = press_g(State::default(), 'm');
        let (state, _) = press(state, 'p');
        assert!(
            !state.monitor.paused,
            "connecting is not yet a feed to pause"
        );

        let token = state.monitor.feed_token;
        let (state, _) = update(state, Msg::FeedOpened { token });
        let (state, _) = press(state, 'p');
        assert!(state.monitor.paused);
    }

    #[test]
    fn r_reopens_a_closed_feed_without_resetting_the_tail() {
        let (state, _) = press_g(State::default(), 'm');
        let token = state.monitor.feed_token;
        let (mut state, _) = update(state, Msg::FeedOpened { token });
        state.monitor.push_monitor_line(1, "kept".into());
        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token,
                reason: Some("server closed it".into()),
            },
        );
        assert_eq!(
            state.monitor.status,
            FeedStatus::Closed {
                reason: Some("server closed it".into())
            }
        );

        let (state, cmds) = press(state, 'r');
        assert_eq!(state.monitor.status, FeedStatus::Connecting);
        assert_eq!(state.monitor.len(), 1, "the buffer is not reset by reopen");
        assert_ne!(state.monitor.feed_token, token);
        assert!(matches!(cmds.as_slice(), [Command::OpenFeed { .. }]));
    }

    #[test]
    fn r_is_a_no_op_while_the_feed_is_already_open() {
        let (state, _) = press_g(State::default(), 'm');
        let token = state.monitor.feed_token;
        let (state, _) = update(state, Msg::FeedOpened { token });
        let (state, cmds) = press(state, 'r');
        assert_eq!(state.monitor.feed_token, token, "no new dial");
        assert!(cmds.is_empty());
    }

    #[test]
    fn copy_reads_the_command_portion_not_the_whole_raw_line() {
        let (state, _) = press_g(State::default(), 'm');
        let token = state.monitor.feed_token;
        let (state, _) = update(
            state,
            Msg::MonitorLine {
                token,
                at_ms: 1,
                raw: r#"1.0 [0 127.0.0.1:1] "set" "k" "v""#.to_string(),
            },
        );
        let (_, cmds) = press(state, 'c');
        match cmds.as_slice() {
            [Command::CopyToClipboard { text, .. }] => {
                assert_eq!(text, r#""set" "k" "v""#);
            }
            other => panic!("expected a copy, got {other:?}"),
        }
    }

    #[test]
    fn filter_narrows_display_only_never_shrinking_the_buffer() {
        let (state, _) = press_g(State::default(), 'm');
        let (state, _) = press(state, '/');
        assert!(state.filtering);
        let token = state.monitor.feed_token;
        let (mut state, _) = update(
            state,
            Msg::MonitorLine {
                token,
                at_ms: 1,
                raw: "one".into(),
            },
        );
        for c in ['x', 'y', 'z'] {
            let (s, _) = press(state, c);
            state = s;
        }
        assert_eq!(state.monitor.filter, "xyz");
        assert_eq!(
            state.monitor.len(),
            1,
            "the filter narrows what's shown, never what's buffered"
        );
    }
}
