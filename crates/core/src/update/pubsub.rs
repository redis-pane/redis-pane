//! The Pub/Sub view (`g p`, R6.2, M3 task 5, `docs/plans/m3-pubsub.md`):
//! opening it (lazily, decision 6), closing the feed on every way out,
//! subscribing/unsubscribing, strip/tail focus and chip navigation, the
//! add-input capture, movement/pause/filter/copy/reopen on the tail, and
//! folding an arriving `Msg::PubSubMessage` into the bounded tail.
//!
//! Switching into this view touches `State::screen` and `State::pubsub`,
//! same as `update::monitor`/`update::slowlog` touch their own — never
//! `State::open`/`State::link`, so a round trip through Pub/Sub never
//! disarms tracking on the Open key.

use super::*;
use crate::command::FeedKindMsg;
use crate::state::{FeedStatus, PubSubFocus, Subscription, View, parse_subscription};

/// How many messages one `PageUp`/`PageDown` moves the selection — the same
/// fixed figure Monitor's own `MONITOR_PAGE_ROWS` uses, for the same reason
/// (no `State::visible_rows()` geometry for a full-screen view, ADR-0011).
pub(super) const PUBSUB_PAGE_ROWS: usize = 10;

/// `g p`: open the Pub/Sub view. Never stages a confirmation (decision 7 —
/// unlike Monitor, subscribing costs only what the reader chose). Opens
/// lazily (decision 6): a remembered, non-empty subscription list redials
/// immediately; an empty one dials nothing and opens the add-input focused
/// instead. Reachable from any screen, including from inside the Pub/Sub
/// view itself — pressing `g p` again always closes whatever feed was open
/// and reopens against the same remembered list, the same "always the same
/// thing happens" rule `open_monitor_view` follows, except the list itself
/// (decision 3) is not reset the way Monitor's tail is.
pub(super) fn open_pubsub(mut state: State) -> (State, Vec<Command>) {
    let mut commands = leave_slowlog(&mut state);
    commands.extend(leave_monitor(&mut state));
    commands.extend(leave_pubsub(&mut state));
    commands.extend(leave_dashboard(&mut state));
    state.screen = View::PubSub;
    state.pubsub.open_fresh();
    if state.pubsub.subscriptions.is_empty() {
        // Decision 6: dial nothing, focus the add-input instead.
        state.pubsub.adding = true;
    } else {
        let token = issue_feed_token(&mut state);
        state.pubsub.feed_token = token;
        state.pubsub.status = FeedStatus::Connecting;
        commands.push(Command::OpenFeed {
            kind: FeedKindMsg::Subscribe(state.pubsub.subscriptions.clone()),
            token,
        });
    }
    (state, commands)
}

/// Close whatever feed the Pub/Sub view has open, if any — Monitor's
/// `leave_monitor`, mirrored. A no-op unless the feed is actually
/// `Connecting`/`Open`, so leaving a screen that was never Pub/Sub (or an
/// already-`Idle`/`Closed` one) never emits a spurious `Command::CloseFeed`.
/// `state.pubsub.subscriptions` is untouched (decision 3) — only the
/// connection goes away, never the remembered list.
pub(super) fn leave_pubsub(state: &mut State) -> Vec<Command> {
    if matches!(
        state.pubsub.status,
        FeedStatus::Connecting | FeedStatus::Open
    ) {
        state.pubsub.status = FeedStatus::Idle;
        vec![Command::CloseFeed]
    } else {
        Vec::new()
    }
}

/// `r` while the Pub/Sub view is showing a closed feed: reopen against the
/// full remembered subscription list (Monitor's `reopen_monitor_feed`,
/// mirrored) — a no-op while already `Connecting`/`Open`, or with nothing
/// remembered to reopen (there is no feed a reader could mean by `r` before
/// the first subscription exists; that case is `g p`'s lazy-open, not a
/// reopen).
pub(super) fn reopen_pubsub_feed(mut state: State) -> (State, Vec<Command>) {
    if !matches!(state.pubsub.status, FeedStatus::Closed { .. }) {
        return (state, Vec::new());
    }
    if state.pubsub.subscriptions.is_empty() {
        return (state, Vec::new());
    }
    let token = issue_feed_token(&mut state);
    state.pubsub.feed_token = token;
    state.pubsub.status = FeedStatus::Connecting;
    let subs = state.pubsub.subscriptions.clone();
    (
        state,
        vec![Command::OpenFeed {
            kind: FeedKindMsg::Subscribe(subs),
            token,
        }],
    )
}

/// Add `sub` to the remembered list and get it flowing — the three cases:
/// nothing subscribed yet (dial fresh, decision 6's lazy open resolving),
/// a live feed (`Command::UpdateSubscription` on the same connection, never
/// a close-then-reopen — `m3-pubsub.md`'s "unsubscribing cleanly" section's
/// sibling concern for *adding* without disturbing what is already
/// flowing), or a feed the server closed on us (redial the whole list
/// rather than trying to append to a dead connection). Already-subscribed
/// is a silent no-op, not a duplicate chip — the reader asking to subscribe
/// to something already subscribed has nothing left to do.
fn add_subscription(state: &mut State, sub: Subscription) -> Vec<Command> {
    if state.pubsub.subscriptions.contains(&sub) {
        return Vec::new();
    }
    state.pubsub.subscriptions.push(sub.clone());
    match state.pubsub.status {
        FeedStatus::Idle | FeedStatus::Closed { .. } => {
            let token = issue_feed_token(state);
            state.pubsub.feed_token = token;
            state.pubsub.status = FeedStatus::Connecting;
            let subs = state.pubsub.subscriptions.clone();
            vec![Command::OpenFeed {
                kind: FeedKindMsg::Subscribe(subs),
                token,
            }]
        }
        FeedStatus::Connecting | FeedStatus::Open => vec![Command::UpdateSubscription {
            add: vec![sub],
            remove: Vec::new(),
        }],
    }
}

/// Remove the subscription at `idx` — the chip's own `d` (decision 1).
/// Removed from the remembered list immediately, optimistically: the reader
/// asked to unsubscribe, so the chip is gone whether or not the server's own
/// `UNSUBSCRIBE`/`PUNSUBSCRIBE` (issued only while the feed is actually
/// live) succeeds. A failure surfaces as an R7.4 notification
/// (`Msg::SubscriptionFailed`) but there is nothing to restore — the chip
/// was already gone locally the moment the reader pressed `d`.
fn remove_subscription_at(state: &mut State, idx: usize) -> Vec<Command> {
    if idx >= state.pubsub.subscriptions.len() {
        return Vec::new();
    }
    let sub = state.pubsub.subscriptions.remove(idx);
    if state.pubsub.selected_chip >= state.pubsub.subscriptions.len() {
        state.pubsub.selected_chip = state.pubsub.subscriptions.len().saturating_sub(1);
    }
    match state.pubsub.status {
        FeedStatus::Connecting | FeedStatus::Open => vec![Command::UpdateSubscription {
            add: Vec::new(),
            remove: vec![sub],
        }],
        FeedStatus::Idle | FeedStatus::Closed { .. } => Vec::new(),
    }
}

/// `←`/`→` on the subscription strip (decision 1). A no-op with nothing to
/// move over.
fn move_chip(state: &mut State, delta: isize) {
    if state.pubsub.subscriptions.is_empty() {
        return;
    }
    let max = (state.pubsub.subscriptions.len() - 1) as isize;
    let new = (state.pubsub.selected_chip as isize + delta).clamp(0, max);
    state.pubsub.selected_chip = new as usize;
}

/// Movement, focus, chip editing, pause, reopen, copy and filter, scoped to
/// the Pub/Sub view — `dispatch_action`'s own guard routes here for exactly
/// the actions this view gives a meaning to.
pub(super) fn pubsub_dispatch(mut state: State, action: Action) -> (State, Vec<Command>) {
    match action {
        // `Tab`: strip ↔ tail (decision 1) — the Pub/Sub-view meaning of the
        // key that means "keys pane ↔ Viewer" everywhere else, the same
        // per-view reuse `Action::Copy`/`Action::Delete`/`Action::Filter`
        // already get across Monitor/Slowlog/the keys pane.
        Action::CyclePane => {
            state.pubsub.focus = match state.pubsub.focus {
                PubSubFocus::Strip => PubSubFocus::Tail,
                PubSubFocus::Tail => PubSubFocus::Strip,
            };
            (state, Vec::new())
        }
        // `a`: open the add-input from either half — the strip advertises
        // `a add` whichever has focus, and the tail gives `a` no other
        // meaning to collide with. Reuses `Action::Add`, bound to `a`
        // everywhere else for "add a key field" — the same key, a different
        // meaning under a different screen.
        Action::Add => {
            state.pubsub.adding = true;
            state.pubsub.input.clear();
            (state, Vec::new())
        }
        // `d`: unsubscribe the selected chip, strip-scoped.
        Action::Delete if state.pubsub.focus == PubSubFocus::Strip => {
            let idx = state.pubsub.selected_chip;
            let commands = remove_subscription_at(&mut state, idx);
            (state, commands)
        }
        // `→`/`←`: move the selected chip, strip-scoped. Reuses
        // `Action::Open`/`Action::CollapseGroup` — the keys pane's own
        // Right/Left actions — the same per-view-reuse rule as `CyclePane`
        // above.
        Action::Open if state.pubsub.focus == PubSubFocus::Strip => {
            move_chip(&mut state, 1);
            (state, Vec::new())
        }
        Action::CollapseGroup if state.pubsub.focus == PubSubFocus::Strip => {
            move_chip(&mut state, -1);
            (state, Vec::new())
        }
        // `r`: reopen a closed feed — available regardless of which half has
        // focus, the same as Monitor's `r`.
        Action::Refetch => reopen_pubsub_feed(state),
        // Everything else is tail-scoped and a no-op on the strip — moving,
        // pausing or filtering a tail nobody is looking at has nothing to
        // show for it, the same "no-op, not reinterpreted" shape
        // `monitor_dispatch` follows for its own guards.
        Action::MoveDown if state.pubsub.focus == PubSubFocus::Tail => {
            state.pubsub.move_selection(1);
            (state, Vec::new())
        }
        Action::MoveUp if state.pubsub.focus == PubSubFocus::Tail => {
            state.pubsub.move_selection(-1);
            (state, Vec::new())
        }
        Action::PageDown if state.pubsub.focus == PubSubFocus::Tail => {
            state.pubsub.move_selection(PUBSUB_PAGE_ROWS as isize);
            (state, Vec::new())
        }
        Action::PageUp if state.pubsub.focus == PubSubFocus::Tail => {
            state.pubsub.move_selection(-(PUBSUB_PAGE_ROWS as isize));
            (state, Vec::new())
        }
        Action::Top if state.pubsub.focus == PubSubFocus::Tail => {
            state.pubsub.to_top();
            (state, Vec::new())
        }
        Action::Bottom if state.pubsub.focus == PubSubFocus::Tail => {
            state.pubsub.to_bottom();
            (state, Vec::new())
        }
        Action::TogglePause
            if state.pubsub.focus == PubSubFocus::Tail
                && state.pubsub.status == FeedStatus::Open =>
        {
            state.pubsub.toggle_pause();
            (state, Vec::new())
        }
        Action::TogglePause => (state, Vec::new()),
        Action::Copy if state.pubsub.focus == PubSubFocus::Tail => pubsub_copy(state),
        Action::Filter if state.pubsub.focus == PubSubFocus::Tail => {
            state.filtering = true;
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// `c` on the tail: copy the selected message's payload, run through the
/// Viewer's own byte-escaping — the same `cell_text` Monitor's `c` and the
/// Slowlog view's own copy already use.
fn pubsub_copy(state: State) -> (State, Vec<Command>) {
    let Some(msg) = state.pubsub.selected_message() else {
        return (state, Vec::new());
    };
    let text = crate::state::value::cell_text(&msg.payload);
    (
        state,
        vec![Command::CopyToClipboard {
            text,
            label: "payload".into(),
        }],
    )
}

/// Keys typed while the add-input is capturing (`a`, decision 1) — the same
/// modal-capture shape the keys pane's own filter and Monitor's filter use,
/// but building `PubSubState::input` and, on `Enter`, parsing and
/// submitting it as a subscription rather than narrowing a display.
pub(super) fn pubsub_add_key(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    match key.code {
        KeyCode::Esc => {
            state.pubsub.adding = false;
            state.pubsub.input.clear();
            (state, Vec::new())
        }
        KeyCode::Enter => {
            let text = state.pubsub.input.trim().to_string();
            state.pubsub.adding = false;
            state.pubsub.input.clear();
            if text.is_empty() {
                return (state, Vec::new());
            }
            let sub = parse_subscription(&text);
            let commands = add_subscription(&mut state, sub);
            (state, commands)
        }
        KeyCode::Backspace => {
            state.pubsub.input.pop();
            (state, Vec::new())
        }
        KeyCode::Char(c) if !key.ctrl && !key.alt => {
            state.pubsub.input.push(c);
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// Keys typed while the tail's filter is capturing (`/`, decision 8) — the
/// same modal capture UX Monitor's `monitor_filter_key` uses, narrowing
/// `PubSubState::filter` for display only: channel and payload, matched at
/// render time, never what gets buffered.
pub(super) fn pubsub_filter_key(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    match key.code {
        KeyCode::Esc => {
            state.filtering = false;
            state.pubsub.filter.clear();
            (state, Vec::new())
        }
        KeyCode::Enter => {
            state.filtering = false;
            (state, Vec::new())
        }
        KeyCode::Backspace => {
            state.pubsub.filter.pop();
            (state, Vec::new())
        }
        KeyCode::Char(c) if !key.ctrl && !key.alt => {
            state.pubsub.filter.push(c);
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// `Msg::PubSubMessage`: fold a message from the feed into the bounded tail.
///
/// Guarded twice, exactly like Monitor's `monitor_line`: `token` must still
/// name the feed currently open, and the Pub/Sub view must still be
/// showing — closing the feed on the way out (`leave_pubsub`) does not
/// itself change the token, so a handful of messages already in flight when
/// `Command::CloseFeed` was issued could still arrive with a token that
/// matches; this second check is what stops them from growing a buffer
/// nobody is looking at.
pub(super) fn pubsub_message(
    mut state: State,
    token: crate::command::FeedToken,
    at_ms: u64,
    channel: Vec<u8>,
    via: Option<Vec<u8>>,
    payload: Vec<u8>,
) -> (State, Vec<Command>) {
    if token == state.pubsub.feed_token && state.screen == View::PubSub {
        state
            .pubsub
            .push_pubsub_message(at_ms, channel, via, payload);
    }
    (state, Vec::new())
}

/// `Msg::SubscriptionFailed`: a `Command::UpdateSubscription` add or remove
/// failed on the server (R7.4 — CLAUDE.md: never a silently dropped `Err`).
///
/// For an add failure, `subs` must not stay shown as subscribed — the
/// server never actually subscribed them — so they are dropped from
/// `state.pubsub.subscriptions` here; for a remove failure the chip was
/// already removed locally when the remove was requested
/// (`remove_subscription_at`), so there is nothing to undo and this is
/// notification-only. Either way the failure is surfaced the same way every
/// other failed command is (`update::failed`'s sibling for this specific
/// message type, since Pub/Sub's failure needs to touch `state.pubsub` too,
/// which the generic `Msg::Failed` handler has no reason to know about).
pub(super) fn subscription_failed(
    mut state: State,
    command: String,
    detail: String,
    at_ms: u64,
    subs: Vec<Subscription>,
) -> (State, Vec<Command>) {
    state.pubsub.subscriptions.retain(|s| !subs.contains(s));
    if state.pubsub.selected_chip >= state.pubsub.subscriptions.len() {
        state.pubsub.selected_chip = state.pubsub.subscriptions.len().saturating_sub(1);
    }
    state.error = Some((format!("{command}: {detail}"), at_ms));
    (state, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Msg;

    fn press(state: State, c: char) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
    }

    fn press_key(state: State, code: KeyCode) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(code)))
    }

    fn press_g(state: State, second: char) -> (State, Vec<Command>) {
        let (state, _) = press(state, 'g');
        press(state, second)
    }

    // ── Opening ──────────────────────────────────────────────────────────

    #[test]
    fn g_p_with_no_remembered_subscriptions_opens_with_the_add_input_focused_and_dials_nothing() {
        let (state, cmds) = press_g(State::default(), 'p');
        assert_eq!(state.screen, View::PubSub);
        assert!(state.pubsub.adding, "decision 6: the add input opens");
        assert_eq!(state.pubsub.status, FeedStatus::Idle);
        assert!(
            cmds.is_empty(),
            "nothing dials until the first subscription"
        );
    }

    #[test]
    fn g_p_with_a_remembered_subscription_redials_immediately() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, cmds) = press_g(state, 'p');
        assert_eq!(state.screen, View::PubSub);
        assert!(!state.pubsub.adding);
        assert_eq!(state.pubsub.status, FeedStatus::Connecting);
        assert!(matches!(
            cmds.as_slice(),
            [Command::OpenFeed {
                kind: FeedKindMsg::Subscribe(subs),
                ..
            }] if subs == &[Subscription::Channel("orders".into())]
        ));
    }

    #[test]
    fn no_confirmation_is_ever_staged_opening_pubsub_in_any_environment() {
        use crate::state::{Connection, Environment};
        for env in [
            Environment::Local,
            Environment::Staging,
            Environment::Prod,
            Environment::Unknown,
        ] {
            let state = State {
                connection: Connection {
                    environment: env,
                    ..Connection::default()
                },
                ..State::default()
            };
            let (state, _) = press_g(state, 'p');
            assert_eq!(state.screen, View::PubSub, "{env:?}");
            assert!(state.pending_feed.is_none(), "{env:?}");
        }
    }

    #[test]
    fn esc_from_pubsub_closes_the_feed_and_returns_to_keys() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let token = state.pubsub.feed_token;
        let (state, _) = update(state, Msg::FeedOpened { token });
        assert_eq!(state.pubsub.status, FeedStatus::Open);

        let (state, cmds) = press_key(state, KeyCode::Esc);
        assert_eq!(state.screen, View::Keys);
        assert_eq!(state.pubsub.status, FeedStatus::Idle);
        assert_eq!(cmds, vec![Command::CloseFeed]);
        assert_eq!(
            state.pubsub.subscriptions,
            vec![Subscription::Channel("orders".into())],
            "leaving the view keeps the remembered list (decision 3)"
        );
    }

    #[test]
    fn g_k_from_pubsub_closes_the_feed_too() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let (state, cmds) = press_g(state, 'k');
        assert_eq!(state.screen, View::Keys);
        assert_eq!(cmds, vec![Command::CloseFeed]);
    }

    #[test]
    fn quitting_from_pubsub_closes_the_feed_too() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let (state, cmds) = press(state, 'q');
        assert!(state.quitting);
        assert!(cmds.contains(&Command::CloseFeed));
        assert!(cmds.contains(&Command::Quit));
    }

    #[test]
    fn opening_monitor_from_pubsub_closes_the_pubsub_feed() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let (state, cmds) = press_g(state, 'm');
        assert_eq!(state.screen, View::Monitor);
        assert!(cmds.contains(&Command::CloseFeed));
    }

    #[test]
    fn opening_pubsub_from_monitor_closes_the_monitor_feed() {
        let (state, _) = press_g(State::default(), 'm');
        let (state, cmds) = press_g(state, 'p');
        assert_eq!(state.screen, View::PubSub);
        assert!(cmds.contains(&Command::CloseFeed));
    }

    // ── The add-input ───────────────────────────────────────────────────

    #[test]
    fn typing_and_submitting_a_channel_subscribes_and_dials() {
        let (state, _) = press_g(State::default(), 'p');
        assert!(state.pubsub.adding);
        let mut state = state;
        for c in "orders".chars() {
            let (s, _) = press(state, c);
            state = s;
        }
        assert_eq!(state.pubsub.input, "orders");
        let (state, cmds) = press_key(state, KeyCode::Enter);
        assert!(!state.pubsub.adding);
        assert!(state.pubsub.input.is_empty());
        assert_eq!(
            state.pubsub.subscriptions,
            vec![Subscription::Channel("orders".into())]
        );
        assert!(matches!(cmds.as_slice(), [Command::OpenFeed { .. }]));
    }

    #[test]
    fn a_pattern_is_auto_detected_from_the_typed_glob() {
        let (state, _) = press_g(State::default(), 'p');
        let mut state = state;
        for c in "user:*".chars() {
            let (s, _) = press(state, c);
            state = s;
        }
        let (state, _) = press_key(state, KeyCode::Enter);
        assert_eq!(
            state.pubsub.subscriptions,
            vec![Subscription::Pattern("user:*".into())]
        );
    }

    #[test]
    fn esc_on_the_add_input_discards_the_typed_text_without_subscribing() {
        let (state, _) = press_g(State::default(), 'p');
        let mut state = state;
        for c in "orders".chars() {
            let (s, _) = press(state, c);
            state = s;
        }
        let (state, cmds) = press_key(state, KeyCode::Esc);
        assert!(!state.pubsub.adding);
        assert!(state.pubsub.input.is_empty());
        assert!(state.pubsub.subscriptions.is_empty());
        assert!(cmds.is_empty());
    }

    #[test]
    fn submitting_empty_input_does_nothing() {
        let (state, _) = press_g(State::default(), 'p');
        let (state, cmds) = press_key(state, KeyCode::Enter);
        assert!(!state.pubsub.adding, "the input still closes");
        assert!(state.pubsub.subscriptions.is_empty());
        assert!(cmds.is_empty());
    }

    #[test]
    fn adding_a_second_channel_while_the_feed_is_open_updates_rather_than_reopens() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("first".into())];
        let (state, _) = press_g(state, 'p');
        let token = state.pubsub.feed_token;
        let (state, _) = update(state, Msg::FeedOpened { token });

        let (state, _) = press(state, 'a');
        let mut state = state;
        for c in "second".chars() {
            let (s, _) = press(state, c);
            state = s;
        }
        let (state, cmds) = press_key(state, KeyCode::Enter);
        assert_eq!(state.pubsub.feed_token, token, "no new dial");
        assert_eq!(
            cmds,
            vec![Command::UpdateSubscription {
                add: vec![Subscription::Channel("second".into())],
                remove: Vec::new(),
            }]
        );
        assert_eq!(state.pubsub.subscriptions.len(), 2);
    }

    #[test]
    fn subscribing_to_something_already_subscribed_is_a_silent_no_op() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let token = state.pubsub.feed_token;
        let (state, _) = update(state, Msg::FeedOpened { token });

        let (state, _) = press(state, 'a');
        let mut state = state;
        for c in "orders".chars() {
            let (s, _) = press(state, c);
            state = s;
        }
        let (state, cmds) = press_key(state, KeyCode::Enter);
        assert_eq!(state.pubsub.subscriptions.len(), 1);
        assert!(cmds.is_empty());
    }

    // ── Chips: focus, navigation, removal ───────────────────────────────

    #[test]
    fn tab_flips_strip_and_tail_focus() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        assert!(
            !state.pubsub.adding,
            "a remembered subscription skips decision 6's lazy open"
        );
        assert_eq!(
            state.pubsub.focus,
            PubSubFocus::Tail,
            "opens on the tail, where the messages are"
        );
        let (state, _) = press_key(state, KeyCode::Tab);
        assert_eq!(state.pubsub.focus, PubSubFocus::Strip);
        let (state, _) = press_key(state, KeyCode::Tab);
        assert_eq!(state.pubsub.focus, PubSubFocus::Tail);
    }

    #[test]
    fn a_opens_the_add_input_from_the_tail_too() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        assert_eq!(state.pubsub.focus, PubSubFocus::Tail);
        let (state, _) = press_key(state, KeyCode::Char('a'));
        assert!(
            state.pubsub.adding,
            "the strip advertises `a add` either way"
        );
    }

    fn with_two_chips_on_strip() -> State {
        let mut state = State {
            screen: View::PubSub,
            ..State::default()
        };
        state.pubsub.subscriptions = vec![
            Subscription::Channel("first".into()),
            Subscription::Channel("second".into()),
        ];
        state.pubsub.focus = PubSubFocus::Strip;
        state
    }

    #[test]
    fn right_and_left_move_the_selected_chip_and_clamp() {
        let state = with_two_chips_on_strip();
        assert_eq!(state.pubsub.selected_chip, 0);
        let (state, _) = press_key(state, KeyCode::Right);
        assert_eq!(state.pubsub.selected_chip, 1);
        let (state, _) = press_key(state, KeyCode::Right);
        assert_eq!(state.pubsub.selected_chip, 1, "clamped at the last chip");
        let (state, _) = press_key(state, KeyCode::Left);
        assert_eq!(state.pubsub.selected_chip, 0);
        let (state, _) = press_key(state, KeyCode::Left);
        assert_eq!(state.pubsub.selected_chip, 0, "clamped at the first chip");
    }

    #[test]
    fn d_unsubscribes_the_selected_chip_and_clamps_selection() {
        let mut state = with_two_chips_on_strip();
        state.pubsub.selected_chip = 1;
        let (state, cmds) = press(state, 'd');
        assert_eq!(
            state.pubsub.subscriptions,
            vec![Subscription::Channel("first".into())]
        );
        assert_eq!(state.pubsub.selected_chip, 0);
        // Feed was never opened (Idle), so nothing to unsubscribe server-side.
        assert!(cmds.is_empty());
    }

    #[test]
    fn d_on_an_open_feed_issues_an_unsubscribe() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![
            Subscription::Channel("first".into()),
            Subscription::Channel("second".into()),
        ];
        let (state, _) = press_g(state, 'p');
        let token = state.pubsub.feed_token;
        let (mut state, _) = update(state, Msg::FeedOpened { token });
        state.pubsub.focus = PubSubFocus::Strip;
        state.pubsub.selected_chip = 0;

        let (state, cmds) = press(state, 'd');
        assert_eq!(
            state.pubsub.subscriptions,
            vec![Subscription::Channel("second".into())]
        );
        assert_eq!(
            cmds,
            vec![Command::UpdateSubscription {
                add: Vec::new(),
                remove: vec![Subscription::Channel("first".into())],
            }]
        );
    }

    // ── The tail: movement, pause, filter, copy, reopen ─────────────────

    fn with_open_tail() -> State {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let token = state.pubsub.feed_token;
        let (mut state, _) = update(state, Msg::FeedOpened { token });
        state.pubsub.focus = PubSubFocus::Tail;
        state
    }

    #[test]
    fn tail_actions_are_no_ops_while_the_strip_has_focus() {
        let mut state = with_open_tail();
        state.pubsub.focus = PubSubFocus::Strip;
        let token = state.pubsub.feed_token;
        let (state, _) = update(
            state,
            Msg::PubSubMessage {
                token,
                at_ms: 1,
                channel: b"orders".to_vec(),
                via: None,
                payload: b"one".to_vec(),
            },
        );
        let (state, _) = press(state, 'p');
        assert!(!state.pubsub.paused, "p is tail-scoped, strip has focus");
    }

    #[test]
    fn p_toggles_pause_only_on_the_tail_while_open() {
        let state = with_open_tail();
        let (state, _) = press(state, 'p');
        assert!(state.pubsub.paused);
    }

    #[test]
    fn c_copies_the_selected_messages_payload() {
        let state = with_open_tail();
        let token = state.pubsub.feed_token;
        let (state, _) = update(
            state,
            Msg::PubSubMessage {
                token,
                at_ms: 1,
                channel: b"orders".to_vec(),
                via: None,
                payload: b"hello".to_vec(),
            },
        );
        let (_, cmds) = press(state, 'c');
        match cmds.as_slice() {
            [Command::CopyToClipboard { text, .. }] => assert_eq!(text, "hello"),
            other => panic!("expected a copy, got {other:?}"),
        }
    }

    #[test]
    fn filter_narrows_display_only_never_shrinking_the_buffer() {
        let state = with_open_tail();
        let (state, _) = press(state, '/');
        assert!(state.filtering);
        let token = state.pubsub.feed_token;
        let (mut state, _) = update(
            state,
            Msg::PubSubMessage {
                token,
                at_ms: 1,
                channel: b"orders".to_vec(),
                via: None,
                payload: b"one".to_vec(),
            },
        );
        for c in ['x', 'y', 'z'] {
            let (s, _) = press(state, c);
            state = s;
        }
        assert_eq!(state.pubsub.filter, "xyz");
        assert_eq!(state.pubsub.messages().len(), 1);
    }

    #[test]
    fn r_reopens_a_closed_feed_with_the_full_remembered_list() {
        let mut state = with_open_tail();
        state
            .pubsub
            .subscriptions
            .push(Subscription::Pattern("evt:*".into()));
        let token = state.pubsub.feed_token;
        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token,
                reason: Some("server closed it".into()),
            },
        );
        let (state, cmds) = press(state, 'r');
        assert_eq!(state.pubsub.status, FeedStatus::Connecting);
        assert_ne!(state.pubsub.feed_token, token);
        assert!(matches!(
            cmds.as_slice(),
            [Command::OpenFeed { kind: FeedKindMsg::Subscribe(subs), .. }]
                if subs.len() == 2
        ));
    }

    // ── Msg::PubSubMessage folding ───────────────────────────────────────

    #[test]
    fn a_pubsub_message_folds_into_the_buffer_when_the_token_matches_and_the_view_is_showing() {
        let state = with_open_tail();
        let token = state.pubsub.feed_token;
        let (state, _) = update(
            state,
            Msg::PubSubMessage {
                token,
                at_ms: 1,
                channel: b"orders".to_vec(),
                via: None,
                payload: b"hi".to_vec(),
            },
        );
        assert_eq!(state.pubsub.messages().len(), 1);
    }

    #[test]
    fn a_pubsub_message_after_leaving_the_view_is_dropped_even_with_a_matching_token() {
        let state = with_open_tail();
        let token = state.pubsub.feed_token;
        let (state, _) = press_key(state, KeyCode::Esc);
        assert_eq!(state.screen, View::Keys);
        let (state, _) = update(
            state,
            Msg::PubSubMessage {
                token,
                at_ms: 1,
                channel: b"orders".to_vec(),
                via: None,
                payload: b"arrived too late".to_vec(),
            },
        );
        assert!(state.pubsub.messages().is_empty());
    }

    #[test]
    fn a_stale_pubsub_message_is_dropped() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = press_g(state, 'p');
        let stale = state.pubsub.feed_token;
        // Supersede with a fresh `g p`.
        let (state, _) = press_g(state, 'p');
        let (state, _) = update(
            state,
            Msg::PubSubMessage {
                token: stale,
                at_ms: 1,
                channel: b"orders".to_vec(),
                via: None,
                payload: b"stale".to_vec(),
            },
        );
        assert!(state.pubsub.messages().is_empty());
    }

    #[test]
    fn a_pmessage_populates_both_the_matched_pattern_and_the_concrete_channel() {
        let state = with_open_tail();
        let token = state.pubsub.feed_token;
        let (state, _) = update(
            state,
            Msg::PubSubMessage {
                token,
                at_ms: 1,
                channel: b"user:42".to_vec(),
                via: Some(b"user:*".to_vec()),
                payload: b"hi".to_vec(),
            },
        );
        let msg = state.pubsub.messages().back().unwrap();
        assert_eq!(msg.channel, b"user:42");
        assert_eq!(msg.via.as_deref(), Some(b"user:*".as_slice()));
    }

    // ── Msg::SubscriptionFailed ──────────────────────────────────────────

    #[test]
    fn a_failed_subscribe_drops_the_chip_and_raises_a_notification() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![
            Subscription::Channel("allowed".into()),
            Subscription::Channel("denied".into()),
        ];
        let (state, _) = update(
            state,
            Msg::SubscriptionFailed {
                command: "SUBSCRIBE".into(),
                detail: "NOPERM this user has no permissions to access one of the channels used as arguments".into(),
                at_ms: 1,
                subs: vec![Subscription::Channel("denied".into())],
            },
        );
        assert_eq!(
            state.pubsub.subscriptions,
            vec![Subscription::Channel("allowed".into())],
            "a chip that never actually subscribed must not stay shown as subscribed"
        );
        let shown = state.error_text().expect("the failure must be visible");
        assert!(shown.contains("SUBSCRIBE"), "{shown}");
        assert!(shown.contains("NOPERM"), "{shown}");
    }

    #[test]
    fn a_failed_unsubscribe_is_notification_only_since_the_chip_is_already_gone() {
        let mut state = State::default();
        state.pubsub.subscriptions = vec![Subscription::Channel("orders".into())];
        let (state, _) = update(
            state,
            Msg::SubscriptionFailed {
                command: "UNSUBSCRIBE".into(),
                detail: "connection reset".into(),
                at_ms: 1,
                subs: vec![Subscription::Channel("orders".into())],
            },
        );
        // The chip was never in `subs` to begin with in this scenario (it was
        // already removed locally by `remove_subscription_at` before the
        // command was even issued) — this only proves the notification
        // still raises even though there was nothing left to drop.
        let shown = state.error_text().expect("the failure must be visible");
        assert!(shown.contains("UNSUBSCRIBE"), "{shown}");
    }
}
