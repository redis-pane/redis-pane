//! The single entry point into the core (PLAN M0.4).

use ratatui_textarea::CursorMove;

use crate::command::{FeedToken, ReadToken};
use crate::key::KeyName;
use crate::keymap::Action;
use crate::msg::KeyCode;
use crate::msg::KeyPress;
use crate::msg::MouseAction;
use crate::mutation::{Mutation, MutationOutcome, NotWritten};
use crate::render::layout::{self, Pane};
use crate::state::copy::{CopyWhat, redis_cli_command, value_text};
use crate::state::value::Value;
use crate::state::{
    Attachment, EditBuffer, EditTarget, FieldPart, Link, OpenKey, PendingMutation, PendingRead,
    ReadOnlyReason, ScanState, SortBy, Tracking, View, is_valid_zset_score,
};
use crate::{Command, Msg, State};

mod confirm;
mod dashboard;
mod editor;
mod feed;
mod keys;
mod link;
mod monitor;
mod mouse;
mod pubsub;
mod scan;
mod slowlog;
mod viewer;

// Every submodule is `pub(super)`, never `pub`: `update()` stays the only way
// into this module from outside it (the split plan's invariant 1). These
// globs exist so a test module's `use super::*` keeps resolving every helper
// it used before the split — see `docs/plans/review-m2-update-module-split.md`,
// "The trick that makes this low-risk". A child module (a `#[cfg(test)]` mod
// below) can see a private `use` in its parent, so nothing here needs `pub`.
use self::confirm::*;
use self::dashboard::*;
use self::editor::*;
use self::feed::*;
use self::keys::*;
use self::link::*;
use self::monitor::*;
use self::mouse::*;
use self::pubsub::*;
use self::scan::*;
use self::slowlog::*;
use self::viewer::*;

/// Mint the token for a read about to be issued, superseding any in flight.
///
/// Every read goes through here, for the same reason every read goes through
/// one `Command`: a read issued without bumping the token would be answered by
/// a reply the core could not tell apart from a stale one.
fn issue_read(state: &mut State) -> ReadToken {
    state.read_token = state.read_token.next();
    state.read_token
}

/// Mint the token for a feed connection about to be opened — Monitor
/// (`g m`) and Pub/Sub (`g p`) alike share this one counter
/// (`state.feed_token_seq`), rather than each minting from its own
/// `feed_token` field the way an earlier version of this did. Two
/// independent per-feature counters can hold the same numeric value (both
/// start at the same default), which made `Msg::FeedOpened`/`Msg::FeedClosed`
/// routing correct only *because* leaving one feature's view always sets its
/// `status` to `Idle` before the other's feed can open — an ordering
/// invariant elsewhere in the code, not a property of the token itself. One
/// shared counter makes a `FeedToken` name exactly one feed, ever, so routing
/// no longer depends on that invariant holding.
fn issue_feed_token(state: &mut State) -> FeedToken {
    state.feed_token_seq = state.feed_token_seq.next();
    state.feed_token_seq
}

/// Issue a Refetch of the Open key: mint its token, record it as the pending
/// read so the frame can say a read is in flight until the reply lands, and
/// return the one [`Command::ReadKey`] that asks for it.
///
/// With nothing open there is nothing to refetch: no command, and no token
/// spent superseding a read nobody issued.
fn refetch(state: &mut State) -> Vec<Command> {
    let Some(open) = &state.open else {
        return Vec::new();
    };
    let (key, index) = (open.name.clone(), open.index);
    let token = issue_read(state);
    state.open_pending = Some(PendingRead {
        name: key.clone(),
        token,
        index,
        issued_at_ms: None,
        activate_cursor: false,
        own_write: false,
    });
    vec![read_key(state, key, index, token)]
}

/// The one read command, with the arming decision filled in from the core's
/// own view of the link (ADR-0006, review H3). Every read is built here.
fn read_key(state: &State, key: KeyName, index: Option<usize>, token: ReadToken) -> Command {
    Command::ReadKey {
        key,
        index,
        token,
        arm: state.read_arms_tracking(),
    }
}

/// The epoch-seconds reading [`crate::state::loaded::LoadedSet::set_ttl`]
/// stores, from the epoch-milliseconds a `Msg` carries. One place to do this
/// conversion rather than two, so both cannot quietly disagree about it.
fn epoch_secs(at_ms: u64) -> u32 {
    (at_ms / 1000) as u32
}

/// `Msg::ReadIssued`: timestamp the pending read for the loading indicator's
/// delay gate. A token that no longer names the outstanding read — already
/// superseded, or already answered — is ignored, the same discipline
/// `ValueLoaded`/`ValueGone` already apply to a stale token.
fn read_issued(mut state: State, token: ReadToken, at_ms: u64) -> (State, Vec<Command>) {
    if let Some(pending) = &mut state.open_pending
        && pending.token == token
    {
        pending.issued_at_ms = Some(at_ms);
    }
    (state, Vec::new())
}

/// `Msg::Paste`: a bracketed paste (ADR-0014). Routed by which capture mode,
/// if any, is active — the inline editor's name part, its body, or the
/// filter — rather than owned by either `editor` or `keys`, since it is the
/// one message that can land in either.
fn paste(mut state: State, text: String) -> (State, Vec<Command>) {
    if let Some(editor) = state.open.as_mut().and_then(OpenKey::typing_mut) {
        // `is_single_line_capture` in place of `active_part() ==
        // Some(FieldPart::Name)` (PLAN M2 task 10, D11, ADR-0019): a TTL
        // capture has no `FieldPart` to be on, so the old predicate would
        // have sent a pasted duration into the buffer's unused `TextArea`
        // instead of its own `text` — the same routing gap `editor_key`'s
        // top check had, one call site over.
        if editor.is_single_line_capture() {
            let stripped: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
            editor.name_push_str(&stripped);
        } else {
            editor.insert_str(&text);
        }
        (state, Vec::new())
    } else if state.filtering {
        let stripped: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        state.list.filter.push_str(&stripped);
        keys::filter_edited(state)
    } else {
        (state, Vec::new())
    }
}

/// Takes a message, returns new state plus commands for a shell to execute.
///
/// Pure: no I/O, no clock, no randomness. Time arrives inside the message
/// (see [`Msg::ReadCompleted`]) rather than being read here, which is what
/// keeps a frame a function of state alone (ADR-0011).
pub fn update(state: State, msg: Msg) -> (State, Vec<Command>) {
    let (mut state, commands) = step(state, msg);
    // The value cursor lives only in a focused value pane (issue #55). Many
    // routes move focus to the keys pane — `Tab`, a click or scroll there,
    // `Esc` while an `Enter` read is still in flight and its reply asks for
    // the cursor — and each of them once left `cursor_active` behind: the
    // hint bar described the key list, the arrows drove the value cursor,
    // and `Esc` saw focus already on Keys and had nothing to pop. Enforced
    // here, after every message, so no single route can forget it again.
    if state.focus != Pane::Value
        && let Some(open) = &mut state.open
    {
        open.cursor_active = false;
    }
    (state, commands)
}

/// Everything [`update`] does except the focus invariant it enforces once
/// `step` has returned.
fn step(mut state: State, msg: Msg) -> (State, Vec<Command>) {
    match msg {
        Msg::Key(key) => key_press(state, key),
        Msg::Mouse(action) => mouse_action(state, action),
        Msg::Resized { cols, rows } => {
            state.cols = cols;
            state.rows = rows;
            state.rewrap_open();
            // A filter being typed must stay where it can be seen. Narrowing
            // the terminal past two panes with the Viewer focused would
            // otherwise leave the capture running inside a pane that is no
            // longer drawn — the same invisible keystroke sink `/` used to open
            // directly, arrived at by dragging a window edge instead.
            if state.filtering && !state.keys_pane_visible() {
                state.focus = Pane::Keys;
            }
            (state, Vec::new())
        }
        Msg::ReadCompleted { at_ms } => {
            state.last_read_ms = Some(at_ms);
            (state, Vec::new())
        }
        Msg::ReadIssued { token, at_ms } => read_issued(state, token, at_ms),
        Msg::Connected {
            version,
            tracking_supported,
        } => connected(state, version, tracking_supported),
        Msg::TopologyChanged(topology) => {
            state.connection.topology = Some(topology);
            (state, Vec::new())
        }
        Msg::ConnectionLost => connection_lost(state),
        Msg::ReconnectScheduled {
            attempt,
            retry_in_ms,
        } => reconnect_scheduled(state, attempt, retry_in_ms),
        Msg::TrackingArmed => tracking_armed(state),
        Msg::Invalidated => invalidated(state),
        Msg::ScanStarted { estimated_total } => scan_started(state, estimated_total),
        Msg::ScanBatch { keys } => scan_batch(state, keys),
        Msg::MetadataBatch {
            entries,
            gone,
            at_ms,
            epoch,
        } => metadata_batch(state, entries, gone, at_ms, epoch),
        Msg::ScanComplete => scan_complete(state),
        Msg::ScanCancelled => scan_cancelled(state),
        Msg::ScanFailed { error } => scan_failed(state, error),
        Msg::ScanInterrupted { reason } => scan_interrupted(state, reason),
        Msg::ValueLoaded {
            token,
            index,
            name,
            value,
            ttl_seconds,
            size_bytes,
            at_ms,
        } => value_loaded(
            state,
            ValueRead {
                token,
                index,
                name,
                value,
                ttl_seconds,
                size_bytes,
                at_ms,
            },
        ),
        Msg::ValueGone {
            token,
            index,
            name,
            at_ms,
        } => value_gone(state, token, index, name, at_ms),
        Msg::Copied { label, at_ms } => {
            state.notice = Some((format!("copied {label}"), at_ms));
            (state, Vec::new())
        }
        Msg::Noticed { text, at_ms } => {
            state.notice = Some((text, at_ms));
            (state, Vec::new())
        }
        Msg::ServerState {
            read_only,
            condition,
        } => server_state(state, read_only, condition),
        Msg::Failed {
            command,
            detail,
            at_ms,
        } => failed(state, command, detail, at_ms),
        Msg::Paste(text) => paste(state, text),
        Msg::MutationSettled {
            mutation,
            index,
            result,
            at_ms,
        } => mutation_settled(state, mutation, index, result, at_ms),
        Msg::Quit => quit(state),
        Msg::SlowlogLoaded { entries } => slowlog_loaded(state, entries),
        Msg::SlowlogFailed { detail, at_ms } => slowlog_failed(state, detail, at_ms),
        Msg::ServerInfoLoaded { info, at_ms, token } => {
            server_info_loaded(state, info, at_ms, token)
        }
        Msg::ServerInfoFailed {
            detail,
            at_ms,
            token,
        } => server_info_failed(state, detail, at_ms, token),
        Msg::ClusterInfoLoaded {
            nodes,
            health,
            at_ms,
            token,
        } => cluster_info_loaded(state, nodes, health, at_ms, token),
        Msg::DashboardPollTick => dashboard_poll_tick(state),
        Msg::FilterRebuildDue => keys::filter_rebuild_due(state),
        Msg::FeedOpened { token } => feed_opened(state, token),
        Msg::FeedClosed { token, reason } => feed_closed(state, token, reason),
        Msg::MonitorLine { token, at_ms, raw } => monitor_line(state, token, at_ms, raw),
        Msg::PubSubMessage {
            token,
            at_ms,
            channel,
            via,
            payload,
        } => pubsub_message(state, token, at_ms, channel, via, payload),
        Msg::SubscriptionFailed {
            command,
            detail,
            at_ms,
            subs,
        } => subscription_failed(state, command, detail, at_ms, subs),
    }
}

/// Which input mode is active. Derived from `State`, never stored: two
/// fields that must agree are two fields that can disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The contextual help overlay is open (M3). Ranked first — above
    /// Confirm — because help drawn over a confirm dialog is on top, so it
    /// owns the keys: `Esc`/`?`/`F1` close it and `Tab` flips the viewed
    /// pane, and every other keypress (`y`, a stray `d`) must be a no-op
    /// rather than reaching the dialog underneath (decision 8,
    /// `docs/plans/m3-contextual-help.md`: "pressing `d` to 'see what
    /// happens' must not stage a delete").
    Help,
    Confirm,
    Editing,
    Filtering,
    /// The Pub/Sub view's add-input is capturing text (`a`, or opened
    /// automatically by decision 6's lazy open). A distinct mode from
    /// `Filtering`: different key (`a` vs `/`), different target
    /// (`PubSubState::input`, submitted as a subscription, vs
    /// `PubSubState::filter`, narrowing a display), never active at the
    /// same time as each other.
    PubSubAdding,
    Normal,
}

/// The one place mode precedence is decided — help overlay, then confirm
/// dialog, then editor, then filter, then Normal.
pub(crate) fn mode(state: &State) -> Mode {
    if state.help.is_some() {
        return Mode::Help;
    }
    mode_beneath_help(state)
}

/// `mode`'s precedence with the help overlay set aside — what
/// [`crate::help::context`] describes (help is drawn *over* a context, not
/// instead of one: closing it must reveal exactly the mode that was showing
/// before it opened), and what `mode` itself falls back to once Help is
/// ruled out. Never returns [`Mode::Help`].
pub(crate) fn mode_beneath_help(state: &State) -> Mode {
    // A staged mutation is a modal dialog: it is the only thing on screen
    // that can act on the keypress until it is confirmed or dismissed,
    // exactly as `state.filtering`, below, is the only thing capturing text.
    // Read-only Mode is decided *here*, at confirm — never at the keypress
    // that staged the mutation — so the preview always shows the real
    // command and its blast radius before the reader learns whether they
    // are allowed to run it (R4.4, DESIGN §6.5).
    // `pending_feed` is Monitor's own confirmation (decision 3,
    // `docs/plans/m3-monitor.md`) — a distinct kind from `PendingMutation`,
    // never routed through the mutation chokepoint below, but the same modal
    // "only this captures the keypress" rule applies.
    if state.confirm.is_some() || state.pending_feed.is_some() {
        return Mode::Confirm;
    }
    // The inline editor is a mode of its own too (ADR-0014), ranked above the
    // filter: both capture ordinary characters as text, and a key open for
    // editing outranks a filter that could only have been left running in
    // the background.
    if state.open.as_ref().and_then(OpenKey::typing).is_some() {
        return Mode::Editing;
    }
    // While the filter is capturing, ordinary characters are text rather than
    // commands. Only Esc and Enter mean anything else.
    if state.filtering {
        return Mode::Filtering;
    }
    // The Pub/Sub add-input, ranked with the same "captures ordinary
    // characters as text" precedence as Filtering — checked after it since
    // the two can never coexist (`state.filtering` is tail-scoped, `adding`
    // is strip-scoped) and order between them is therefore never observable,
    // only documentation of which was written first.
    if state.pubsub.adding {
        return Mode::PubSubAdding;
    }
    Mode::Normal
}

/// Resolve a keypress through the keymap, never against hard-coded keys.
///
/// The hint bar and help overlay read the same map, so what is shown is always
/// the effective binding after user overrides (R7.5).
fn key_press(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    let current_mode = mode(&state);
    // `F1` opens help from every mode (decision 5) — checked here, before
    // Confirm/Editing/Filtering get first crack at the key, since each of
    // those hardcodes its own vocabulary rather than resolving through the
    // keymap and would otherwise swallow it (a stray `y`, a typed
    // character). Resolved through the keymap (`Action::Help`'s effective
    // binding), not a hard-coded action — only the *shape* of the key
    // (`KeyCode::F(_)`) is hard-coded, to tell it apart from `?`, which stays
    // Normal-mode-only (`?` is a typed character in Filtering/Editing and is
    // swallowed in Confirm — decision 5: "`?` keeps working wherever it does
    // today (Normal mode)"). Normal mode needs no special case: `F1` already
    // resolves to `Action::Help` there through the ordinary path below.
    if matches!(
        current_mode,
        Mode::Confirm | Mode::Editing | Mode::Filtering | Mode::PubSubAdding
    ) && matches!(key.code, KeyCode::F(_))
        && state.keymap.action_for(&key) == Some(Action::Help)
    {
        return open_help(state);
    }
    match current_mode {
        Mode::Help => return help_key(state, key),
        Mode::Confirm => {
            // Monitor's own confirmation (`pending_feed`) is checked first:
            // the two pending kinds are mutually exclusive in practice (a
            // mutation preview and the `MONITOR` cost dialog have no shared
            // trigger), but checking here rather than assuming keeps that
            // true by construction instead of by convention.
            if let Some(kind) = state.pending_feed.take() {
                return feed_confirm_key(state, kind, key);
            }
            // `confirm_key` needs the `PendingMutation` by value: `mode` only
            // answers which mode is active, so the take() still happens here.
            let pending = state.confirm.take().expect("Mode::Confirm implies confirm");
            return confirm_key(state, pending, key);
        }
        Mode::Editing => return editor_key(state, key),
        Mode::Filtering => return filter_key(state, key),
        Mode::PubSubAdding => return pubsub_add_key(state, key),
        Mode::Normal => {}
    }
    // `g`-chords (DESIGN §3's jump list, M3): a pending prefix waits, with no
    // timeout, for exactly one more keypress. `Esc` or a second key that
    // names no chord clears it and does nothing else — the same "swallowed,
    // not reinterpreted" rule every other modal capture in this app follows
    // (Filter's `Esc`, Confirm's stray keys). Checked only in `Mode::Normal`:
    // Filtering/Editing/Confirm each hardcode their own vocabulary and reach
    // `g` before this ever would, so a pending chord can never coexist with
    // one of them.
    if let Some(prefix) = state.pending_chord.take() {
        if key.code == KeyCode::Esc {
            return (state, Vec::new());
        }
        return match state.keymap.chord_action(&prefix, &key) {
            Some(action) => dispatch_action(state, action),
            None => (state, Vec::new()),
        };
    }
    if state.keymap.is_chord_prefix(&key) {
        state.pending_chord = Some(key);
        return (state, Vec::new());
    }
    let Some(action) = state.keymap.action_for(&key) else {
        return (state, Vec::new());
    };
    // An action whose pane is off screen does nothing. Below 70 columns only
    // one pane is drawn, so without this `↓` moves a cursor nobody can see and
    // `/` starts capturing into a filter line of zero width — every keypress
    // after it vanishing, `q` typing a `q`, the app indistinguishable from
    // hung. Stack navigation says the other pane is simply not there right now
    // (DESIGN §2), and the honest reading of "not there" is "does nothing",
    // not "does something invisible". `Esc` is the way back and it is in the
    // hint bar. Above 70 columns both panes are drawn and nothing changes.
    if !action.pane_is_on_screen(&state) {
        return (state, Vec::new());
    }
    dispatch_action(state, action)
}

/// Open help, viewing whichever pane is currently focused — `HelpView::pane`
/// starts there and `Tab` flips it (decision 7). Shared by `Action::Help`'s
/// Normal-mode dispatch and the `F1` shortcut above, so there is exactly one
/// place that decides what "open help" means.
fn open_help(mut state: State) -> (State, Vec<Command>) {
    state.help = Some(crate::state::HelpView { pane: state.focus });
    (state, Vec::new())
}

/// Keys read while help is open (Mode::Help, ranked first): `Esc`, `?` and
/// `F1` all close it; `Tab` flips which pane's context is viewed, but only in
/// the two contexts that have another pane to point at
/// (`crate::help::HelpContext::Keys`/`Value` — decision 7: "Only offered for
/// the two pane contexts"); every other key is a no-op — help is modal
/// (decision 8), so a stray `d` must not reach the dialog this is drawn over.
fn help_key(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    match key.code {
        KeyCode::Esc => {
            state.help = None;
        }
        KeyCode::Char('?') if !key.ctrl && !key.alt => {
            state.help = None;
        }
        KeyCode::F(1) => {
            state.help = None;
        }
        KeyCode::Tab => {
            let ctx = crate::help::context(&state);
            if matches!(
                ctx,
                crate::help::HelpContext::Keys { .. } | crate::help::HelpContext::Value(_)
            ) && let Some(view) = &mut state.help
            {
                view.pane = match view.pane {
                    Pane::Keys => Pane::Value,
                    Pane::Value => Pane::Keys,
                };
            }
        }
        _ => {}
    }
    (state, Vec::new())
}

/// What a resolved `Action` actually does — the one dispatch step `key_press`
/// (above) calls once the mode gate and the `pane_is_on_screen` gate have
/// both passed, kept separate from `key_press` itself so a resolved `Action`
/// has exactly one place that decides what it does.
fn dispatch_action(mut state: State, action: Action) -> (State, Vec<Command>) {
    // The Slowlog view is a full screen, not the two-pane browser: only the
    // actions it gives its own meaning to (movement, sort, refetch, copy,
    // `d` staging `RESET` — PLAN decision 3) and the handful that are the
    // app's, not a pane's (Quit/Help/ToggleReadOnly/Cancel/the two view
    // chords) do anything while it is showing. Every keys/value-pane action
    // — Edit, Add, Filter, ToggleTree, CyclePane, the split-resize pair,
    // EnterValueCursor, the editor-scoped four — would otherwise reach
    // straight into `State::keys`/`State::open` underneath a screen that is
    // not showing either of them, which is exactly the kind of invisible
    // action `key_press`'s `pane_is_on_screen` gate exists to rule out one
    // level up.
    if state.screen == View::Slowlog {
        return match action {
            Action::Quit => quit(state),
            Action::Help => open_help(state),
            Action::Cancel => cancel(state),
            Action::ToggleReadOnly => toggle_read_only(state),
            Action::OpenKeysView => open_keys_view(state),
            Action::OpenSlowlog => open_slowlog(state),
            Action::OpenMonitor => open_monitor(state),
            Action::OpenPubSub => open_pubsub(state),
            Action::OpenDashboard => open_dashboard(state),
            Action::MoveDown
            | Action::MoveUp
            | Action::PageDown
            | Action::PageUp
            | Action::Top
            | Action::Bottom
            | Action::Sort
            | Action::Refetch
            | Action::Copy
            | Action::Delete => slowlog_dispatch(state, action),
            _ => (state, Vec::new()),
        };
    }
    // The Monitor view (M3 phase B, `docs/plans/m3-monitor.md`) is the same
    // shape one level over: a full screen, not the two-pane browser, so only
    // the actions it gives its own meaning to (movement/following, pause,
    // reopen, copy, filter — decisions 5–9) and the app-level handful do
    // anything while it is showing.
    if state.screen == View::Monitor {
        return match action {
            Action::Quit => quit(state),
            Action::Help => open_help(state),
            Action::Cancel => cancel(state),
            Action::ToggleReadOnly => toggle_read_only(state),
            Action::OpenKeysView => open_keys_view(state),
            Action::OpenSlowlog => open_slowlog(state),
            Action::OpenMonitor => open_monitor(state),
            Action::OpenPubSub => open_pubsub(state),
            Action::OpenDashboard => open_dashboard(state),
            Action::MoveDown
            | Action::MoveUp
            | Action::PageDown
            | Action::PageUp
            | Action::Top
            | Action::Bottom
            | Action::TogglePause
            | Action::Refetch
            | Action::Copy
            | Action::Filter => monitor_dispatch(state, action),
            _ => (state, Vec::new()),
        };
    }
    // The Pub/Sub view (M3 task 5, `docs/plans/m3-pubsub.md`) is the same
    // shape again: a full screen, so only the actions it gives its own
    // meaning to (strip/tail focus, chip add/remove/navigate, tail
    // movement/pause/filter/copy, reopen — decision 1) and the app-level
    // handful do anything while it is showing.
    if state.screen == View::PubSub {
        return match action {
            Action::Quit => quit(state),
            Action::Help => open_help(state),
            Action::Cancel => cancel(state),
            Action::ToggleReadOnly => toggle_read_only(state),
            Action::OpenKeysView => open_keys_view(state),
            Action::OpenSlowlog => open_slowlog(state),
            Action::OpenMonitor => open_monitor(state),
            Action::OpenPubSub => open_pubsub(state),
            Action::OpenDashboard => open_dashboard(state),
            Action::MoveDown
            | Action::MoveUp
            | Action::PageDown
            | Action::PageUp
            | Action::Top
            | Action::Bottom
            | Action::TogglePause
            | Action::Refetch
            | Action::Copy
            | Action::Filter
            | Action::CyclePane
            | Action::Add
            | Action::Delete
            | Action::Open
            | Action::CollapseGroup => pubsub_dispatch(state, action),
            _ => (state, Vec::new()),
        };
    }
    // The Dashboard view (M3 task 6, `docs/plans/m3-dashboard.md`) is the
    // same shape once more: a full screen, so only the actions it gives its
    // own meaning to (tile-focus movement, the raw-`INFO` overlay, refetch,
    // copy — decision 6) and the app-level handful do anything while it is
    // showing.
    if state.screen == View::Dashboard {
        return match action {
            Action::Quit => quit(state),
            Action::Help => open_help(state),
            Action::Cancel => cancel(state),
            Action::ToggleReadOnly => toggle_read_only(state),
            Action::OpenKeysView => open_keys_view(state),
            Action::OpenSlowlog => open_slowlog(state),
            Action::OpenMonitor => open_monitor(state),
            Action::OpenPubSub => open_pubsub(state),
            Action::OpenDashboard => open_dashboard(state),
            Action::MoveUp
            | Action::MoveDown
            | Action::Open
            | Action::CollapseGroup
            | Action::EnterValueCursor
            | Action::Copy
            | Action::Refetch => dashboard_dispatch(state, action),
            _ => (state, Vec::new()),
        };
    }
    match action {
        Action::Quit => quit(state),
        Action::OpenKeysView => open_keys_view(state),
        Action::OpenSlowlog => open_slowlog(state),
        Action::OpenMonitor => open_monitor(state),
        Action::OpenPubSub => open_pubsub(state),
        Action::OpenDashboard => open_dashboard(state),
        // Scoped to the Monitor view alone — see the block above. A no-op
        // everywhere else, the same as `Action::Sort`'s bare `s` is a no-op
        // wherever nothing gives it a meaning.
        Action::TogglePause => (state, Vec::new()),
        // Reached only from `Mode::Normal` (`key_press`'s mode match), which
        // itself only happens when `state.help` is `None` — `mode` ranks
        // `Mode::Help` first, so this is always an open, never a toggle-off;
        // `help_key` is what closes it.
        Action::Help => open_help(state),
        Action::Cancel => cancel(state),
        Action::Refetch => refetch_action(state),
        Action::CyclePane => {
            // Focus only ever moves to a pane there is something to focus.
            // With no key open the Viewer has nothing in it, and below 70
            // columns moving focus is what draws the other pane — so focusing
            // an empty Viewer there would black out the screen.
            state.focus = match state.focus {
                Pane::Keys if state.open.is_some() => Pane::Value,
                Pane::Keys => Pane::Keys,
                Pane::Value => Pane::Keys,
            };
            (state, Vec::new())
        }
        Action::WidenKeysPane => {
            // The clamp lives in `layout()`, which is the only place that
            // knows the terminal's actual width; here the offset is a plain
            // number, so a session that never touches this key costs nothing
            // and the core stays geometry-free (ADR-0011).
            if state.split_is_adjustable() {
                state.split_adjust = state
                    .split_adjust
                    .saturating_add(crate::render::layout::SPLIT_STEP as i16);
                state.rewrap_open();
            }
            (state, Vec::new())
        }
        Action::NarrowKeysPane => {
            if state.split_is_adjustable() {
                state.split_adjust = state
                    .split_adjust
                    .saturating_sub(crate::render::layout::SPLIT_STEP as i16);
                state.rewrap_open();
            }
            (state, Vec::new())
        }
        // `d` is focus-dependent (D4, PLAN M2 task 6), like `c`: the keys
        // pane's Selected-key delete below is unchanged; the Viewer's
        // Hash-field/Set-member/List-element delete (D5, PLAN M2 task 7,
        // ADR-0016; PLAN M2 task 8, ADR-0017) is `delete_value_row`'s job,
        // which tells the three apart itself.
        Action::Delete if state.keys_pane_focused() => delete_selected_key(state),
        Action::Delete => delete_value_row(state),
        // Nothing is staged — `key_press` intercepts every keypress before
        // this match while `state.confirm` is `Some`, so `y` only ever
        // reaches here with nothing to confirm.
        Action::ConfirmMutation => (state, Vec::new()),
        // Each of these six moves the value cursor instead of the key list
        // while cursor mode is active (`Enter`/`Action::EnterValueCursor`) —
        // the same keys, aimed at whichever thing the reader is currently
        // working in, never a silent redirect from merely looking at a pane.
        Action::MoveDown if cursor_active(&state) => move_cursor(state, 1),
        Action::MoveUp if cursor_active(&state) => move_cursor(state, -1),
        Action::PageDown if cursor_active(&state) => move_cursor(state, VALUE_PAGE_ROWS as isize),
        Action::PageUp if cursor_active(&state) => move_cursor(state, -(VALUE_PAGE_ROWS as isize)),
        Action::Top if cursor_active(&state) => cursor_to(state, 0),
        Action::Bottom if cursor_active(&state) => cursor_to(state, usize::MAX),
        Action::MoveDown => move_selection(state, 1),
        Action::MoveUp => move_selection(state, -1),
        Action::PageDown => {
            let page = state.visible_rows() as isize;
            move_selection(state, page)
        }
        Action::PageUp => {
            let page = state.visible_rows() as isize;
            move_selection(state, -page)
        }
        Action::Top => {
            state.view.selected = 0;
            after_move(state)
        }
        Action::Bottom => {
            state.view.selected = state.row_count().saturating_sub(1);
            after_move(state)
        }
        Action::Open => open_selected(state),
        Action::EnterValueCursor => enter_value_cursor(state),
        Action::Copy => {
            // Which pane the reader is looking at decides what `y` copies —
            // no mnemonic to remember, no chord to get half right (DESIGN §4).
            let what = if state.keys_pane_focused() {
                CopyWhat::Key
            } else {
                CopyWhat::Value
            };
            build_copy(state, what)
        }
        Action::CopyCommand => build_copy(state, CopyWhat::Command),
        Action::Edit => open_editor(state),
        Action::Add => begin_add_entry(state),
        // Nothing is open to edit: `key_press` intercepts every keypress
        // before this match while an editor buffer or a field-name capture
        // exists, so these only ever reach here with neither to act on.
        Action::EditorStage | Action::EditorUndo | Action::EditorRedo => (state, Vec::new()),
        Action::Filter => {
            state.filtering = true;
            (state, Vec::new())
        }
        Action::Sort => cycle_sort(state),
        // `t` is focus-dependent, exactly like `d` above (PLAN M2 task 10,
        // D2, ADR-0019): the keys pane's tree toggle is unchanged; the
        // Viewer's TTL editor is `open_ttl_editor`'s job.
        Action::ToggleTree if state.keys_pane_focused() => toggle_tree(state),
        Action::ToggleTree => open_ttl_editor(state),
        Action::CollapseGroup => collapse_group(state),
        Action::ToggleReadOnly => toggle_read_only(state),
    }
}

/// `⌃R`: lift or impose Read-only Mode — shared by the ordinary dispatch
/// above and the Slowlog view's restricted one, since a replica guard must
/// refuse to be lifted (ADR-0009) regardless of which screen is showing.
fn toggle_read_only(mut state: State) -> (State, Vec<Command>) {
    // A replica will refuse writes whatever we believe, so this is not
    // a toggle the user gets to win (ADR-0009).
    match state.read_only {
        Some(ReadOnlyReason::Replica) => {}
        Some(_) => state.read_only = None,
        None => state.read_only = Some(ReadOnlyReason::User),
    }
    (state, Vec::new())
}

/// `Esc`: back out of the nearest thing first — an overlay, then an error,
/// then value-cursor mode, then an in-flight scan. One keypress, one
/// meaning.
fn cancel(mut state: State) -> (State, Vec<Command>) {
    // Help no longer has a branch here: `Mode::Help` outranks `Mode::Normal`
    // (`mode`'s precedence), so `Esc` while help is open never reaches
    // `dispatch_action`/`cancel` at all — `help_key` closes it directly.
    if state.error.is_some() {
        state.error = None;
        return (state, Vec::new());
    }
    // A full-screen view pops back to the browser before anything about the
    // browser's own stack navigation (below) is considered — the same
    // "nearest thing first" rule this whole function follows, one level up
    // from the two-pane split.
    if state.screen == View::Slowlog {
        state.screen = View::Keys;
        return (state, Vec::new());
    }
    // Monitor's own way back — every route out closes the feed first
    // (decision 2, `docs/plans/m3-monitor.md`).
    if state.screen == View::Monitor {
        let commands = leave_monitor(&mut state);
        state.screen = View::Keys;
        return (state, commands);
    }
    // Pub/Sub's own way back — same shape, `docs/plans/m3-pubsub.md`. Note
    // `Esc` while the add-input is capturing never reaches here at all:
    // `Mode::PubSubAdding` routes to `pubsub_add_key` first, which is its
    // own "back out of the nearest thing" (closes the input, not the view).
    if state.screen == View::PubSub {
        let commands = leave_pubsub(&mut state);
        state.screen = View::Keys;
        return (state, commands);
    }
    // The Dashboard's own way back. The raw-`INFO` overlay is the nearest
    // thing when it is open — `Esc` closes it and stops there, exactly the
    // "nearest thing first" rule every other overlay in this app follows —
    // and only a *second* `Esc`, with nothing left to close, leaves the
    // view. No feed to close either way (unlike Monitor/Pub/Sub, `INFO` is
    // request/response on the main connection): leaving only ever flips
    // `state.screen`, and `Msg::DashboardPollTick`'s own guard
    // (`dashboard_poll_tick`) is what actually stops issuing
    // `Command::FetchServerInfo` on the shell's very next tick (decision 1).
    if state.screen == View::Dashboard {
        if state.dashboard.active().expanded_tile.is_some() {
            state.dashboard.active_mut().close_overlay();
            return (state, Vec::new());
        }
        // Inside a node view on a Cluster, `Esc` returns to the overview
        // (M5 task 7); only a second one leaves the view.
        if state.dashboard.drilled() {
            if let Some(c) = state.dashboard.cluster.as_mut() {
                c.undrill();
            }
            return (state, Vec::new());
        }
        // On a Cluster leaving also stops the per-node poll.
        let commands = leave_dashboard(&mut state);
        state.screen = View::Keys;
        return (state, commands);
    }
    // The "pop" half of stack navigation: back to the list you were
    // just looking at, before an unrelated background scan. Also
    // exits value-cursor mode if it was active — the same keypress,
    // since there is nothing left to pop before it. The value stays
    // open and unchanged either way; only where plain movement is
    // aimed changes.
    if state.focus == Pane::Value {
        state.focus = Pane::Keys;
        if let Some(open) = &mut state.open {
            open.cursor_active = false;
        }
        return (state, Vec::new());
    }
    // Every in-flight operation is cancellable (PRD R7.3).
    if state.scan.is_running() {
        return (state, vec![Command::CancelScan]);
    }
    (state, Vec::new())
}

/// `r`: retry a dropped link, rescan the keys pane, apply a held Viewer
/// update, or Refetch the Open key — whichever applies, in that order.
fn refetch_action(mut state: State) -> (State, Vec<Command>) {
    // A dropped link, not merely `Link::Connecting` (before the first
    // connect): there is nothing to refetch or rescan over a dead
    // connection, only a reconnect to retry. ADR-0009: "`r` retries
    // immediately rather than waiting out the timer." Checked before
    // the pane-focus split below, which presupposes a connection to
    // act over.
    //
    // Deliberately narrower than `Liveness::Disconnected`, which also
    // covers `Link::Connecting` — real startup never renders an
    // interactive frame in that state (the shell connects before the
    // event loop starts), but a great many tests build off
    // `State::default()`, whose `Link` defaults to `Connecting`, and
    // never mean to be testing reconnection at all.
    if matches!(state.link, Link::Reconnecting { .. }) {
        return (state, vec![Command::Reconnect { after_ms: 0 }]);
    }
    // `r` acts on the focused pane and nothing else (R2.7), and the
    // hint bar names which half is in force. The keys pane is not
    // push-live — the deletions it can detect for free arrive with the
    // metadata it was already fetching, and anything else needs the
    // keyspace walked again (DESIGN §9).
    //
    // The held-update branch below used to sit *above* this check, so
    // `r` in the keys pane with an update waiting in the Viewer applied
    // that update and did not rescan — while the hint bar said
    // `r rescan`. The list did not move, no scan readout appeared, and
    // a value in the other pane changed instead: no error, no feedback,
    // nothing to explain it. That is the same defect 6d665a3 fixed,
    // surviving in the one branch that ran before the focus check.
    if state.keys_pane_focused() {
        // `None` is the same traversal the session opened with: the
        // scan has never been server-side filtered, `/` narrows the
        // Loaded set on this side, and so the active filter survives a
        // rescan without being mentioned here.
        return (state, vec![Command::StartScan { pattern: None }]);
    }
    // In the Viewer, a held update is the cheapest possible answer:
    // applying what the server has already sent is the one thing `r`
    // must never re-ask for.
    if let Some(open) = &mut state.open
        && open.pending.is_some()
    {
        open.take_pending();
        open.at_rest = true;
        return (state, Vec::new());
    }
    let commands = refetch(&mut state);
    (state, commands)
}

/// An operation failed, shown with the command that failed (R7.4).
fn failed(mut state: State, command: String, detail: String, at_ms: u64) -> (State, Vec<Command>) {
    // A failed read is still a read that answered — its loading indicator
    // would otherwise read `⟳ fetching…` forever, which is exactly the
    // "operation vanishes, nothing on screen explains it" defect the error
    // toast below exists to prevent.
    state.open_pending = None;
    // A failure while a write is in flight (the `SET` was refused) ends the
    // edit, so R3.8's guard is not held long after there is anything left to
    // protect. A buffer still being typed into is not discarded by a failure
    // that has nothing to do with it — a metadata fetch, a clipboard error —
    // which used to switch the guard off under unsaved text (review H2).
    if let Some(open) = &mut state.open
        && open.typing().is_none()
    {
        open.end_edit();
    }
    state.error = Some((format!("{command}: {detail}"), at_ms));
    (state, Vec::new())
}

/// Ends the edit a dialog was confirming, if there is one. Harmless (and a
/// no-op) for mutations that never open an edit, such as `DeleteKey` — safe to
/// call unconditionally from both of `confirm_key`'s non-executing branches.
fn clear_editing(state: &mut State) {
    if let Some(open) = &mut state.open {
        open.end_edit();
    }
}

fn quit(mut state: State) -> (State, Vec<Command>) {
    // Quitting is a route out of the Monitor and Pub/Sub views too
    // (decision 2; `docs/plans/m3-pubsub.md`): a feed left running behind a
    // process that is about to exit costs the server with nothing left to
    // say so, which is exactly the scenario decision 2 exists to rule out.
    let mut commands = leave_monitor(&mut state);
    commands.extend(leave_pubsub(&mut state));
    state.quitting = true;
    commands.push(Command::Quit);
    (state, commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::KeyCode;
    use crate::state::{Environment, Source};

    #[test]
    fn resize_updates_dimensions_and_emits_nothing() {
        let (s, cmds) = update(
            State::default(),
            Msg::Resized {
                cols: 118,
                rows: 29,
            },
        );
        assert_eq!((s.cols, s.rows), (118, 29));
        assert!(cmds.is_empty());
    }

    #[test]
    fn quit_sets_the_flag_and_asks_the_shell_to_exit() {
        let (s, cmds) = update(State::default(), Msg::Quit);
        assert!(s.quitting);
        assert_eq!(cmds, vec![Command::Quit]);
    }

    #[test]
    fn read_completion_records_the_clock_reading_it_was_given() {
        let (s, _) = update(State::default(), Msg::ReadCompleted { at_ms: 9_000 });
        assert_eq!(s.last_read_ms, Some(9_000));
    }

    #[test]
    fn update_is_pure_same_input_same_output() {
        let a = update(State::default(), Msg::Resized { cols: 80, rows: 24 });
        let b = update(State::default(), Msg::Resized { cols: 80, rows: 24 });
        assert_eq!(a, b);
    }

    // ── M0.5: synthetic events drive the core with no terminal attached ─────

    #[test]
    fn q_quits() {
        let (s, cmds) = update(
            State::default(),
            Msg::Key(KeyPress::plain(KeyCode::Char('q'))),
        );
        assert!(s.quitting);
        assert_eq!(cmds, vec![Command::Quit]);
    }

    #[test]
    fn ctrl_c_quits() {
        let (s, cmds) = update(
            State::default(),
            Msg::Key(KeyPress::ctrl(KeyCode::Char('c'))),
        );
        assert!(s.quitting);
        assert_eq!(cmds, vec![Command::Quit]);
    }

    #[test]
    fn ctrl_q_is_not_q() {
        let (s, cmds) = update(
            State::default(),
            Msg::Key(KeyPress::ctrl(KeyCode::Char('q'))),
        );
        assert!(!s.quitting);
        assert!(cmds.is_empty());
    }

    #[test]
    fn an_unbound_key_changes_nothing() {
        let before = State::default();
        let (after, cmds) = update(
            before.clone(),
            Msg::Key(KeyPress::plain(KeyCode::Char('z'))),
        );
        assert_eq!(before, after);
        assert!(cmds.is_empty());
    }

    #[test]
    fn question_mark_opens_the_help_overlay_and_esc_closes_it() {
        let (s, _) = update(
            State::default(),
            Msg::Key(KeyPress::plain(KeyCode::Char('?'))),
        );
        assert!(s.help.is_some());
        let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert!(s.help.is_none());
    }

    #[test]
    fn f1_opens_help_from_normal_mode_too() {
        let (s, _) = update(State::default(), Msg::Key(KeyPress::plain(KeyCode::F(1))));
        assert!(s.help.is_some());
    }

    #[test]
    fn f1_opens_help_from_filter_capture_where_question_mark_only_types() {
        let mut s = State::default();
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char('/'))));
        assert!(s.filtering);
        let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::F(1))));
        assert!(s.help.is_some());
        assert!(s.filtering, "F1 opens help without abandoning the filter");
    }

    #[test]
    fn question_mark_only_types_in_filter_capture_it_does_not_open_help() {
        let mut s = State::default();
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char('/'))));
        let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char('?'))));
        assert!(s.help.is_none());
        assert_eq!(s.list.filter, "?");
    }

    #[test]
    fn help_is_modal_a_stray_d_while_it_is_open_stages_nothing() {
        let mut s = State::default();
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char('?'))));
        assert!(s.help.is_some());
        let (s, cmds) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char('d'))));
        assert!(s.help.is_some(), "still open — `d` is a no-op, not a close");
        assert!(s.confirm.is_none(), "nothing staged");
        assert!(cmds.is_empty());
    }

    #[test]
    fn tab_flips_the_viewed_pane_in_help_without_moving_real_focus() {
        let mut s = viewing();
        assert_eq!(s.focus, Pane::Value);
        (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Char('?'))));
        let view = s.help.expect("help open");
        assert_eq!(view.pane, Pane::Value);

        let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Tab)));
        assert_eq!(
            s.help.expect("still open").pane,
            Pane::Keys,
            "the viewed pane flipped"
        );
        assert_eq!(s.focus, Pane::Value, "real focus did not move");
    }

    /// A key open in the Viewer *and focused*, at a two-pane width — the state
    /// that `Action::Open` produces, since opening a key moves focus onto it.
    fn viewing() -> State {
        State {
            cols: 130,
            rows: 40,
            focus: Pane::Value,
            open: Some(OpenKey::new(
                Some(0),
                "k".into(),
                crate::state::value::Value::Str(crate::state::value::StringValue::new("v", 40)),
                -1,
                10,
                0,
            )),
            ..State::default()
        }
    }

    fn press_r(state: State) -> Vec<Command> {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char('r')))).1
    }

    #[test]
    fn r_in_the_viewer_asks_for_a_refetch_which_is_the_only_read_path() {
        assert!(matches!(
            press_r(viewing()).as_slice(),
            [Command::ReadKey { .. }]
        ));
    }

    /// R2.7: `r` acts on the focused pane and nothing else. This half was
    /// documented from the start and never wired up — the core emitted a
    /// Refetch unconditionally, so `StartScan` was unreachable.
    #[test]
    fn r_in_the_keys_pane_rescans_the_keyspace() {
        let state = State {
            cols: 130,
            rows: 40,
            ..State::default()
        };
        assert_eq!(
            press_r(state),
            vec![Command::StartScan { pattern: None }],
            "with no key open, the keys pane is what `r` acts on"
        );
    }

    /// ADR-0009: "`r` retries immediately rather than waiting out the
    /// timer." Disconnected, there is nothing to refetch or rescan — checked
    /// before the pane-focus split, in both panes, with or without a key open.
    #[test]
    fn r_reconnects_immediately_while_disconnected_regardless_of_pane() {
        fn dropped_link() -> Link {
            Link::Reconnecting {
                attempt: 1,
                retry_in_ms: Some(4_000),
            }
        }
        let keys_pane = State {
            cols: 130,
            rows: 40,
            link: dropped_link(),
            ..State::default()
        };
        assert_eq!(
            press_r(keys_pane),
            vec![Command::Reconnect { after_ms: 0 }],
            "keys pane focused, but there is nothing to rescan without a link"
        );

        let viewer = State {
            link: dropped_link(),
            ..viewing()
        };
        assert_eq!(
            press_r(viewer),
            vec![Command::Reconnect { after_ms: 0 }],
            "Viewer focused, but there is nothing to refetch without a link"
        );
    }

    #[test]
    fn r_follows_focus_at_every_width_not_merely_whether_a_key_is_open() {
        // The bug this replaced: at a two-pane width, focus was inferred from
        // `open.is_some()`. Opening a key then silently handed `r` to the
        // Viewer while the arrow keys still drove the key list, so pressing `r`
        // to rescan quietly refetched the value instead and the list did not
        // move. A key can be open and unfocused; that is what `Tab` is for.
        for cols in [60u16, 130] {
            let focused_value = State { cols, ..viewing() };
            assert!(
                matches!(
                    press_r(focused_value.clone()).as_slice(),
                    [Command::ReadKey { .. }]
                ),
                "at {cols} columns, a focused Viewer owns `r`"
            );

            let focused_keys = State {
                focus: Pane::Keys,
                ..focused_value
            };
            assert_eq!(
                press_r(focused_keys),
                vec![Command::StartScan { pattern: None }],
                "at {cols} columns, an open-but-unfocused key must not own `r`"
            );
        }
    }

    /// PLAN M2 task 10, D2, ADR-0019: `t` splits on focus the same way `r`
    /// does — keys pane still folds/unfolds the tree, value pane opens the
    /// TTL editor. This is the shape the existing focus-dependent `d` test
    /// has, one key over.
    #[test]
    fn t_follows_focus_tree_in_the_keys_pane_ttl_in_the_viewer() {
        let mut before_tree = State {
            focus: Pane::Keys,
            ..viewing()
        };
        assert!(!before_tree.tree_mode);
        let (after_tree, cmds) = update(
            before_tree.clone(),
            Msg::Key(KeyPress::plain(KeyCode::Char('t'))),
        );
        assert!(
            after_tree.tree_mode,
            "t in the keys pane still toggles the tree with a key open"
        );
        assert!(cmds.is_empty());
        assert!(
            after_tree.open.as_ref().unwrap().editor().is_none(),
            "no TTL buffer was opened"
        );
        before_tree.tree_mode = true;

        let (after_ttl, cmds) = update(viewing(), Msg::Key(KeyPress::plain(KeyCode::Char('t'))));
        assert!(
            !after_ttl.tree_mode,
            "t in the value pane must not touch tree mode"
        );
        assert!(cmds.is_empty(), "opening the TTL editor emits no command");
        assert!(
            after_ttl.open.as_ref().unwrap().is_editing(),
            "t in the value pane opens the TTL editor"
        );
        assert!(matches!(
            after_ttl.open.as_ref().unwrap().editor().unwrap().target(),
            crate::state::EditTarget::Ttl { .. }
        ));
    }

    #[test]
    fn tab_moves_focus_between_the_panes_and_r_follows_it() {
        // DESIGN §4 has specified `Tab` since the beginning; nothing
        // implemented it, which left R2.7's "acts on the focused pane" resting
        // on a focus the user could not move.
        let tab = |s: State| update(s, Msg::Key(KeyPress::plain(KeyCode::Tab))).0;

        let s = viewing();
        assert_eq!(s.focus, Pane::Value, "opening a key focuses it");

        let s = tab(s);
        assert_eq!(s.focus, Pane::Keys);
        assert_eq!(
            press_r(s.clone()),
            vec![Command::StartScan { pattern: None }],
            "Tab back to the list must make `r` rescan, without closing the key"
        );
        assert!(s.open.is_some(), "and without closing the key");

        let s = tab(s);
        assert_eq!(s.focus, Pane::Value, "Tab cycles back");
    }

    #[test]
    fn tab_does_not_focus_a_viewer_with_nothing_in_it() {
        // Below 70 columns focusing the Viewer is what *draws* it, so focusing
        // an empty one would blank the screen. Above, it would hand `r` to a
        // pane that has nothing to refetch.
        let s = State {
            cols: 60,
            rows: 40,
            ..State::default()
        };
        let (s, _) = update(s, Msg::Key(KeyPress::plain(KeyCode::Tab)));
        assert_eq!(s.focus, Pane::Keys);
        assert_eq!(press_r(s), vec![Command::StartScan { pattern: None }]);
    }

    // ── the split is resizable (DESIGN §2, UI_TASKS severity 2) ────────────

    #[test]
    fn widen_and_narrow_move_the_split_by_one_step() {
        let state = viewing(); // 130 columns: two panes exist to resize
        let (state, cmds) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Right)));
        assert!(cmds.is_empty(), "a plain state mutation, no command needed");
        assert_eq!(state.split_adjust, crate::render::layout::SPLIT_STEP as i16);

        let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Left)));
        assert_eq!(state.split_adjust, 0, "back where it started");

        let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Left)));
        assert_eq!(
            state.split_adjust,
            -(crate::render::layout::SPLIT_STEP as i16)
        );
    }

    #[test]
    fn resizing_below_seventy_columns_does_nothing() {
        // There is no divider to move with one pane on screen (DESIGN §2) —
        // changing the number would be exactly the invisible action the
        // pane-visibility gate above exists to rule out for every other key.
        let state = State {
            cols: 60,
            rows: 40,
            ..State::default()
        };
        let (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Right)));
        assert_eq!(state.split_adjust, 0);
    }

    #[test]
    fn the_adjustment_survives_moving_around_the_app() {
        // Not reset by anything else the reader does in the same session —
        // opening a key, moving the cursor, changing focus.
        let mut state = viewing();
        (state, _) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Right)));
        let widened = state.split_adjust;
        assert_ne!(widened, 0);

        (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Tab)));
        (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Down)));
        assert_eq!(state.split_adjust, widened, "untouched by ordinary use");
    }

    #[test]
    fn ctrl_r_toggles_read_only_but_a_replica_cannot_be_lifted() {
        // Off -> user-imposed.
        let (s, _) = update(
            State::default(),
            Msg::Key(KeyPress::ctrl(KeyCode::Char('r'))),
        );
        assert_eq!(s.read_only, Some(ReadOnlyReason::User));
        // And back off again.
        let (s, _) = update(s, Msg::Key(KeyPress::ctrl(KeyCode::Char('r'))));
        assert_eq!(s.read_only, None);

        // A replica refuses to be lifted, because the server would refuse the
        // write regardless and a toggle that does nothing is a toggle that lies.
        let replica = State {
            read_only: Some(ReadOnlyReason::Replica),
            ..State::default()
        };
        let (s, _) = update(replica, Msg::Key(KeyPress::ctrl(KeyCode::Char('r'))));
        assert_eq!(s.read_only, Some(ReadOnlyReason::Replica));
    }

    #[test]
    fn a_rebound_key_takes_effect_in_update_not_just_in_the_hint() {
        let mut keymap = crate::keymap::Keymap::default();
        keymap.bind(Action::Quit, KeyPress::ctrl(KeyCode::Char('x')));
        let state = State {
            keymap,
            ..State::default()
        };
        let (s, cmds) = update(state, Msg::Key(KeyPress::ctrl(KeyCode::Char('x'))));
        assert!(s.quitting);
        assert_eq!(cmds, vec![Command::Quit]);
    }

    #[test]
    fn prod_and_unknown_are_read_only_by_default_local_and_staging_are_not() {
        assert!(Environment::Prod.read_only_by_default());
        assert!(Environment::Unknown.read_only_by_default());
        assert!(!Environment::Local.read_only_by_default());
        assert!(!Environment::Staging.read_only_by_default());
    }

    #[test]
    fn source_labels_name_where_the_target_came_from() {
        assert_eq!(Source::Default.label(), "from default");
        assert_eq!(
            Source::Profile("staging".into()).label(),
            "from profile staging"
        );
    }

    fn open_with_live_editor() -> State {
        let value = crate::state::Value::Hash(crate::state::value::PairValue {
            pairs: vec![("f".into(), "v".into())],
            total: 1,
        });
        let mut open = OpenKey::new(Some(0), "k".into(), value, -1, 0, 0);
        open.edit = crate::state::EditPhase::Typing(EditBuffer::new_hash_field());
        State {
            open: Some(open),
            ..State::default()
        }
    }

    #[test]
    fn confirm_outranks_a_live_editor() {
        let mut s = open_with_live_editor();
        s.confirm = Some(PendingMutation::DeleteKey {
            index: 0,
            name: "k".into(),
        });
        assert_eq!(mode(&s), Mode::Confirm);
    }

    #[test]
    fn editor_outranks_the_filter() {
        let mut s = open_with_live_editor();
        s.filtering = true;
        assert_eq!(mode(&s), Mode::Editing);
    }

    #[test]
    fn filter_alone_is_filtering() {
        let s = State {
            filtering: true,
            ..State::default()
        };
        assert_eq!(mode(&s), Mode::Filtering);
    }

    #[test]
    fn bare_state_is_normal() {
        assert_eq!(mode(&State::default()), Mode::Normal);
    }
}

#[cfg(test)]
mod honesty_tests {
    //! The three ways the app could quietly claim something untrue, each with
    //! a test so it cannot come back.

    use super::*;
    use crate::msg::KeyCode;
    use crate::state::{Environment, Liveness, ServerCondition};

    /// R7.4: an error is never swallowed. A Redis failure that produces no
    /// visible effect is indistinguishable from the app deciding to do nothing.
    #[test]
    fn a_failure_surfaces_with_the_command_that_caused_it() {
        let (state, _) = update(
            State::default(),
            Msg::Failed {
                command: "reading user:8812:session".into(),
                detail: "WRONGTYPE Operation against a key".into(),
                at_ms: 1_000,
            },
        );
        let shown = state.error_text().expect("the failure must be visible");
        assert!(shown.contains("reading user:8812:session"), "{shown}");
        assert!(shown.contains("WRONGTYPE"), "{shown}");
    }

    #[test]
    fn an_error_does_not_fade_the_way_a_confirmation_does() {
        // A confirmation nobody reads has cost nothing. An error nobody reads
        // is an error nobody handled.
        let (state, _) = update(
            State::default(),
            Msg::Failed {
                command: "c".into(),
                detail: "d".into(),
                at_ms: 0,
            },
        );
        assert!(state.error_text().is_some());
        assert!(
            state.notice_now(State::NOTICE_MS * 10).is_none(),
            "a copy notice would be gone by now"
        );
        assert!(state.error_text().is_some(), "the error is not");
    }

    #[test]
    fn esc_dismisses_an_error_before_it_touches_anything_else() {
        let (state, _) = update(
            State::default(),
            Msg::Failed {
                command: "c".into(),
                detail: "d".into(),
                at_ms: 0,
            },
        );
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert!(state.error_text().is_none());
    }

    /// ADR-0009: the shell now reports reconnects, and the core must drop the
    /// liveness claim when it does.
    #[test]
    fn a_reported_reconnect_drops_the_liveness_claim_until_re_armed() {
        let viewing = State {
            open: Some(OpenKey::new(
                Some(0),
                "k".into(),
                crate::state::value::Value::Str(crate::state::value::StringValue::new("v", 40)),
                -1,
                1,
                0,
            )),
            ..State::default()
        };
        let (state, _) = update(
            viewing,
            Msg::Connected {
                version: "8.4.0".into(),
                tracking_supported: true,
            },
        );
        let (state, _) = update(state, Msg::TrackingArmed);
        assert_eq!(state.liveness(), Liveness::Live);

        let (state, _) = update(state, Msg::ConnectionLost);
        assert_eq!(state.liveness(), Liveness::Disconnected);

        // The shell re-probes and re-announces; still not live until armed.
        let (state, cmds) = update(
            state,
            Msg::Connected {
                version: "8.4.0".into(),
                tracking_supported: true,
            },
        );
        assert_ne!(state.liveness(), Liveness::Live);
        assert!(cmds.iter().any(|c| matches!(c, Command::ReadKey { .. })));
    }

    /// R1.15: a replica outranks an Environment guard, and cannot be lifted.
    #[test]
    fn a_replica_guard_outranks_an_environment_guard() {
        let state = State {
            read_only: Some(ReadOnlyReason::Environment),
            ..State::default()
        };
        let (state, _) = update(
            state,
            Msg::ServerState {
                read_only: Some(ReadOnlyReason::Replica),
                condition: None,
            },
        );
        assert_eq!(state.read_only, Some(ReadOnlyReason::Replica));
        assert!(!state.read_only_liftable(), "⌃R must not offer to lift it");
    }

    #[test]
    fn a_failover_away_from_a_replica_falls_back_to_the_environment_guard() {
        // Sentinel promotes the replica we were reading. The `replica` reason
        // no longer applies, but a prod Environment guard still does.
        let mut state = State {
            read_only: Some(ReadOnlyReason::Replica),
            ..State::default()
        };
        state.connection.environment = Environment::Prod;
        let (state, _) = update(
            state,
            Msg::ServerState {
                read_only: None,
                condition: None,
            },
        );
        assert_eq!(state.read_only, Some(ReadOnlyReason::Environment));
    }

    #[test]
    fn the_server_changing_its_mind_never_lifts_a_user_imposed_guard() {
        let state = State {
            read_only: Some(ReadOnlyReason::User),
            ..State::default()
        };
        let (state, _) = update(
            state,
            Msg::ServerState {
                read_only: None,
                condition: None,
            },
        );
        assert_eq!(state.read_only, Some(ReadOnlyReason::User));
    }

    #[test]
    fn a_condition_that_rejects_writes_reaches_the_chrome() {
        let (state, _) = update(
            State::default(),
            Msg::ServerState {
                read_only: None,
                condition: Some(ServerCondition::Misconf),
            },
        );
        assert_eq!(state.condition, Some(ServerCondition::Misconf));
        assert!(
            state
                .condition
                .unwrap()
                .readout()
                .contains("writes rejected")
        );
    }
}

#[cfg(test)]
mod stack_navigation_tests {
    //! Below 70 columns there is one pane at a time (DESIGN §2). Opening a
    //! key pushes into it; Esc pops back — a stack two rungs deep, not a full
    //! navigation history, because that is all this layout ever needs.

    use super::*;
    use crate::msg::KeyCode;
    use crate::state::LoadedSet;

    fn browsing() -> State {
        let mut keys = LoadedSet::default();
        keys.push(b"k1");
        keys.push(b"k2");
        let mut state = State {
            keys,
            rows: 30,
            ..State::default()
        };
        state.rebuild_list();
        state
    }

    #[test]
    fn opening_a_key_pushes_into_value_view() {
        let state = browsing();
        assert_eq!(state.focus, Pane::Keys);
        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('l'))));
        assert_eq!(state.focus, Pane::Value);
        assert!(matches!(cmds.first(), Some(Command::ReadKey { .. })));
    }

    #[test]
    fn esc_pops_back_to_keys_from_value_view() {
        let state = browsing();
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('l'))));
        assert_eq!(state.focus, Pane::Value);

        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.focus, Pane::Keys);
        assert!(
            cmds.is_empty(),
            "popping the stack is not itself a scan cancellation"
        );
    }

    #[test]
    fn esc_closes_help_before_popping_the_value_view() {
        // Nearest thing first: an overlay you opened on top of everything
        // closes before backing out of the navigation underneath it.
        let mut state = browsing();
        (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('l'))));
        state.help = Some(crate::state::HelpView { pane: state.focus });

        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert!(state.help.is_none());
        assert_eq!(
            state.focus,
            Pane::Value,
            "one Esc closes one thing, not two"
        );
    }

    #[test]
    fn esc_pops_the_value_view_before_cancelling_an_unrelated_scan() {
        let mut state = browsing();
        (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('l'))));
        (state, _) = update(
            state,
            Msg::ScanStarted {
                estimated_total: 10,
            },
        );
        assert!(state.scan.is_running());

        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.focus, Pane::Keys);
        assert!(
            cmds.is_empty(),
            "the scan must still be running, not cancelled by this Esc"
        );
        assert!(state.scan.is_running());
    }

    #[test]
    fn opening_a_key_at_a_wide_terminal_is_harmless() {
        // The flag is set unconditionally on Open; at any density wider than
        // Single, layout() never consults it, so this must change nothing an
        // actual reader can see.
        let state = browsing();
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('l'))));
        assert_eq!(state.focus, Pane::Value);
        // No assertion on rendered output here — render::layout's own test
        // (`focus_is_ignored_at_any_wider_density`) is the proof;
        // this just confirms the state transition still happens uniformly.
    }
}

#[cfg(test)]
mod topology_tests {
    use super::*;
    use crate::state::Topology;

    #[test]
    fn a_topology_change_updates_the_connection_and_asks_for_nothing() {
        let state = State::default();
        assert_eq!(state.connection.topology, None);
        let topology = Topology {
            primaries: 3,
            nodes: 6,
        };
        let (state, commands) = update(state, Msg::TopologyChanged(topology));
        assert_eq!(state.connection.topology, Some(topology));
        assert!(commands.is_empty());
    }
}

/// M5 task 5 (ADR-0022): on a Cluster the server views that read one node's
/// own figures open onto a notice and issue nothing.
#[cfg(test)]
mod cluster_server_view_tests {
    use super::*;
    use crate::state::{Environment, Topology};

    fn cluster(environment: Environment) -> State {
        let mut state = State::default();
        state.connection.environment = environment;
        state.connection.topology = Some(Topology {
            primaries: 3,
            nodes: 6,
        });
        state
    }

    fn key(state: State, c: char) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
    }

    fn chord(state: State, second: char) -> (State, Vec<Command>) {
        let (state, _) = key(state, 'g');
        key(state, second)
    }

    #[test]
    fn g_s_and_g_m_open_their_views_and_emit_no_fetch_and_no_feed() {
        for env in [
            Environment::Local,
            Environment::Staging,
            Environment::Prod,
            Environment::Unknown,
        ] {
            for (second, view) in [('s', View::Slowlog), ('m', View::Monitor)] {
                let (state, commands) = chord(cluster(env), second);
                assert_eq!(state.screen, view, "{env:?} g {second}");
                assert!(commands.is_empty(), "{env:?} g {second}: {commands:?}");
                assert!(state.pending_feed.is_none(), "no cost dialog for a no-op");
                assert!(!state.slowlog.loading && !state.dashboard.loading);
            }
        }
    }

    #[test]
    fn nothing_in_the_views_issues_anything_on_a_cluster() {
        for second in ['s', 'm'] {
            let (state, _) = chord(cluster(Environment::Staging), second);
            for c in ['r', 'd', 'c', '/', 'p', 'j', 'k'] {
                let (after, commands) = key(state.clone(), c);
                assert!(commands.is_empty(), "g {second} then {c}: {commands:?}");
                assert!(after.confirm.is_none(), "g {second} then {c} staged one");
                assert!(!after.filtering, "g {second} then {c} opened a filter");
            }
            let (_, commands) = update(state, Msg::DashboardPollTick);
            assert!(commands.is_empty());
        }
    }

    #[test]
    fn g_d_on_a_cluster_fetches_every_node_in_every_environment() {
        for env in [
            Environment::Local,
            Environment::Staging,
            Environment::Prod,
            Environment::Unknown,
        ] {
            let (state, commands) = chord(cluster(env), 'd');
            assert_eq!(state.screen, View::Dashboard);
            assert!(state.dashboard.loading);
            assert!(
                matches!(commands.as_slice(), [Command::FetchServerInfo { .. }]),
                "{env:?}: {commands:?}"
            );
            assert!(state.pending_feed.is_none());
        }
    }

    #[test]
    fn navigation_still_leaves_the_view() {
        let (state, _) = chord(cluster(Environment::Staging), 's');
        let (state, _) = chord(state, 'k');
        assert_eq!(state.screen, View::Keys);
        let (state, _) = chord(state, 'd');
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert_eq!(state.screen, View::Keys);
    }

    #[test]
    fn pubsub_still_opens_on_a_cluster() {
        let (state, _) = chord(cluster(Environment::Staging), 'p');
        assert_eq!(state.screen, View::PubSub);
    }

    #[test]
    fn a_single_node_target_still_fetches() {
        let (_, commands) = chord(State::default(), 's');
        assert!(matches!(
            commands.as_slice(),
            [Command::FetchSlowlog { .. }]
        ));
        let (_, commands) = chord(State::default(), 'd');
        assert!(matches!(
            commands.as_slice(),
            [Command::FetchServerInfo { .. }]
        ));
        let (_, commands) = chord(State::default(), 'm');
        assert!(matches!(commands.as_slice(), [Command::OpenFeed { .. }]));
    }
}
