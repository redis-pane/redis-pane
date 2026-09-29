//! Shared handlers for the two feed-connection messages
//! (`docs/plans/m3-feed-connection.md`, M3 task 2 phase A).
//!
//! This is plumbing with no consumer yet: nothing in the keymap emits
//! `Command::OpenFeed` before Monitor (`docs/plans/m3-monitor.md`, phase B)
//! wires `g m` up to it. What lives here is the state machine every feature
//! that opens a feed shares — `FeedOpened`/`FeedClosed` transitioning
//! [`FeedStatus`], and the one guard that keeps a stale shell reply from
//! clobbering a feed the reader has already replaced.

use crate::command::FeedToken;
use crate::state::FeedStatus;
use crate::{Command, State};

/// `Msg::FeedOpened`: the shell's dial succeeded and the feed is streaming.
///
/// Guarded by `token`, the same discipline [`crate::update::read_issued`]
/// applies to `Msg::ReadIssued`: a reply naming a feed that is no longer the
/// one `state.monitor.feed_token` identifies is stale — either the reader left the
/// view (`Command::CloseFeed` already ran, dropping the token's meaning) or a
/// newer `Command::OpenFeed` superseded it — and is a no-op, not a state
/// change.
pub(super) fn feed_opened(mut state: State, token: FeedToken) -> (State, Vec<Command>) {
    // `state.monitor.status != Idle` rules out the sentinel case: `FeedToken::default()`
    // is never handed out by `Command::OpenFeed` (the first minted token is
    // already past it, see `FeedToken::next`), so the only way a message
    // could carry it *and* match `state.monitor.feed_token` is a feed that was never
    // opened — the same "idle, so no-op" rule `feed_closed` states directly.
    if token == state.monitor.feed_token && state.monitor.status != FeedStatus::Idle {
        state.monitor.status = FeedStatus::Open;
    }
    (state, Vec::new())
}

/// `Msg::FeedClosed`: the feed ended, for any reason — the reader asked for
/// it (`Command::CloseFeed`), the server closed it, or the network went
/// silent. Guarded by `token` exactly like [`feed_opened`].
///
/// A `FeedClosed` for a feed that was already `Idle` (never opened, or
/// already closed and the token forgotten) is the same no-op every other
/// stale-reply handler in this module follows — never a panic.
pub(super) fn feed_closed(
    mut state: State,
    token: FeedToken,
    reason: Option<String>,
) -> (State, Vec<Command>) {
    if token == state.monitor.feed_token && state.monitor.status != FeedStatus::Idle {
        state.monitor.status = FeedStatus::Closed { reason };
    }
    (state, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Msg;
    use crate::update::update;

    /// Mint a fresh feed token and set `state.monitor.status` to `Connecting`, the way
    /// `Command::OpenFeed`'s eventual issuer (`g m`, M3 phase B) will —
    /// standing in for that issuer here, since phase A wires no keybinding to
    /// it yet.
    fn connecting() -> (State, FeedToken) {
        let mut state = State::default();
        state.monitor.feed_token = state.monitor.feed_token.next();
        state.monitor.status = FeedStatus::Connecting;
        let token = state.monitor.feed_token;
        (state, token)
    }

    #[test]
    fn feed_opened_moves_idle_to_open() {
        let (state, token) = connecting();
        let (state, cmds) = update(state, Msg::FeedOpened { token });
        assert_eq!(state.monitor.status, FeedStatus::Open);
        assert!(cmds.is_empty());
    }

    #[test]
    fn feed_closed_records_the_reason() {
        let (state, token) = connecting();
        let (state, _) = update(state, Msg::FeedOpened { token });
        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token,
                reason: Some("server closed the connection".into()),
            },
        );
        assert_eq!(
            state.monitor.status,
            FeedStatus::Closed {
                reason: Some("server closed the connection".into())
            }
        );
    }

    #[test]
    fn a_close_the_reader_asked_for_carries_no_reason() {
        let (state, token) = connecting();
        let (state, _) = update(state, Msg::FeedOpened { token });
        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token,
                reason: None,
            },
        );
        assert_eq!(state.monitor.status, FeedStatus::Closed { reason: None });
    }

    /// The staleness guard: a `FeedOpened` for a token that is no longer
    /// `state.monitor.feed_token` — because the reader already closed that feed and a
    /// newer one has since been opened — must not resurrect it as `Open`.
    #[test]
    fn a_stale_feed_opened_is_a_no_op() {
        let (state, old_token) = connecting();
        // A newer feed superseded it.
        let mut state = state;
        state.monitor.feed_token = state.monitor.feed_token.next();
        state.monitor.status = FeedStatus::Connecting;

        let (state, cmds) = update(state, Msg::FeedOpened { token: old_token });
        assert_eq!(
            state.monitor.status,
            FeedStatus::Connecting,
            "a stale token must not move the current feed's status"
        );
        assert!(cmds.is_empty());
    }

    /// The same guard on the close side: a `FeedClosed` for an old token must
    /// not clobber a newer feed that is already `Open`.
    #[test]
    fn a_stale_feed_closed_is_a_no_op() {
        let (state, old_token) = connecting();
        let mut state = state;
        state.monitor.feed_token = state.monitor.feed_token.next();
        let new_token = state.monitor.feed_token;
        state.monitor.status = FeedStatus::Open;

        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token: old_token,
                reason: Some("stale".into()),
            },
        );
        assert_eq!(
            state.monitor.status,
            FeedStatus::Open,
            "a stale token must not close a newer, still-open feed"
        );

        // The real close for the current feed still works.
        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token: new_token,
                reason: None,
            },
        );
        assert_eq!(state.monitor.status, FeedStatus::Closed { reason: None });
    }

    /// `FeedClosed` while idle (never opened) is a no-op, not a panic — the
    /// pattern every other "stale reply" handler in this codebase already
    /// follows (e.g. `open_pending` token mismatches in `update/viewer.rs`).
    #[test]
    fn feed_closed_while_idle_is_a_no_op() {
        let state = State::default();
        let token = state.monitor.feed_token; // the default, unissued token
        let (state, cmds) = update(
            state,
            Msg::FeedClosed {
                token,
                reason: Some("never opened".into()),
            },
        );
        assert_eq!(
            state.monitor.status,
            FeedStatus::Idle,
            "a default token names no feed that was ever opened"
        );
        assert!(cmds.is_empty());
    }
}
