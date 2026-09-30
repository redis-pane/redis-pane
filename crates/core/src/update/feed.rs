//! Shared handlers for the feed-connection lifecycle messages
//! (`docs/plans/m3-feed-connection.md`, M3 task 2 phase A) —
//! `FeedOpened`/`FeedClosed` transitioning [`FeedStatus`], and the one guard
//! that keeps a stale shell reply from clobbering a feed the reader has
//! already replaced.
//!
//! Monitor and Pub/Sub (`docs/plans/m3-pubsub.md`) share these two `Msg`
//! variants — a shell reply about "a feed" says nothing on its own about
//! which feature opened it, only a [`FeedToken`] does. Every `FeedToken` is
//! minted from the one shared counter `state.feed_token_seq`
//! (`update::issue_feed_token`), so a token names exactly one feed, ever: it
//! is never possible for `state.monitor.feed_token` and
//! `state.pubsub.feed_token` to hold the same value at the same time. That
//! is what makes the routing below correct — not an ordering invariant
//! about which feature's `status` happens to be `Idle` when. (An earlier
//! version of this module minted from two independent per-feature counters,
//! which *could* collide; routing correctness then depended on leaving one
//! view always setting its `status` back to `Idle` before the other's feed
//! could open. The shared counter removes that dependency entirely — see
//! `feed_tokens_are_globally_unique_across_features` below.)

use crate::command::FeedToken;
use crate::state::FeedStatus;
use crate::{Command, State};

/// `Msg::FeedOpened`: the shell's dial succeeded and the feed is streaming.
///
/// Guarded by `token`, the same discipline [`crate::update::read_issued`]
/// applies to `Msg::ReadIssued`: a reply naming a feed that is no longer the
/// one the owning feature's `feed_token` identifies is stale — either the
/// reader left the view (`Command::CloseFeed` already ran, dropping the
/// token's meaning) or a newer `Command::OpenFeed` superseded it — and is a
/// no-op, not a state change. Checked against Monitor and Pub/Sub
/// independently (never `else if`): with a globally unique token, at most
/// one of the two conditions can ever be true, so there is nothing to
/// prioritize between them.
pub(super) fn feed_opened(mut state: State, token: FeedToken) -> (State, Vec<Command>) {
    // `status != Idle` guards the "stale within the same feature" case: the
    // sentinel `FeedToken::default()` is never handed out by
    // `Command::OpenFeed` (the first minted token is already past it, see
    // `FeedToken::next`), so the only way a message could carry it *and*
    // match a feature's `feed_token` is a feed that was never opened — the
    // same "idle, so no-op" rule `feed_closed` states directly. It plays no
    // part in telling Monitor and Pub/Sub apart any more — global
    // uniqueness already does that.
    if token == state.monitor.feed_token && state.monitor.status != FeedStatus::Idle {
        state.monitor.status = FeedStatus::Open;
    }
    if token == state.pubsub.feed_token && state.pubsub.status != FeedStatus::Idle {
        state.pubsub.status = FeedStatus::Open;
    }
    (state, Vec::new())
}

/// `Msg::FeedClosed`: the feed ended, for any reason — the reader asked for
/// it (`Command::CloseFeed`), the server closed it, or the network went
/// silent. Guarded by `token` exactly like [`feed_opened`], routed the same
/// way across Monitor and Pub/Sub.
///
/// A `FeedClosed` for a feed that was already `Idle` on both features
/// (never opened, or already closed and the token forgotten) is the same
/// no-op every other stale-reply handler in this module follows — never a
/// panic.
pub(super) fn feed_closed(
    mut state: State,
    token: FeedToken,
    reason: Option<String>,
) -> (State, Vec<Command>) {
    if token == state.monitor.feed_token && state.monitor.status != FeedStatus::Idle {
        state.monitor.status = FeedStatus::Closed {
            reason: reason.clone(),
        };
    }
    if token == state.pubsub.feed_token && state.pubsub.status != FeedStatus::Idle {
        state.pubsub.status = FeedStatus::Closed { reason };
    }
    (state, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Msg;
    use crate::update::{issue_feed_token, update};

    /// Mint a fresh feed token (from the shared counter, `issue_feed_token`)
    /// and set `state.monitor.status` to `Connecting`, the way
    /// `Command::OpenFeed`'s eventual issuer (`g m`) will — standing in for
    /// that issuer here for the handlers that don't need the rest of
    /// `open_monitor_view`'s behaviour.
    fn connecting() -> (State, FeedToken) {
        let mut state = State::default();
        state.monitor.feed_token = issue_feed_token(&mut state);
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
        state.monitor.feed_token = issue_feed_token(&mut state);
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
        state.monitor.feed_token = issue_feed_token(&mut state);
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

    /// The Pub/Sub-side equivalent of [`connecting`] — standing in for
    /// `g p`'s eventual issuer, same as `connecting` stands in for `g m`.
    fn connecting_pubsub() -> (State, FeedToken) {
        let mut state = State::default();
        state.pubsub.feed_token = issue_feed_token(&mut state);
        state.pubsub.status = FeedStatus::Connecting;
        let token = state.pubsub.feed_token;
        (state, token)
    }

    #[test]
    fn a_pubsub_feed_opened_moves_idle_to_open() {
        let (state, token) = connecting_pubsub();
        let (state, cmds) = update(state, Msg::FeedOpened { token });
        assert_eq!(state.pubsub.status, FeedStatus::Open);
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_pubsub_feed_closed_records_the_reason() {
        let (state, token) = connecting_pubsub();
        let (state, _) = update(state, Msg::FeedOpened { token });
        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token,
                reason: Some("server closed the connection".into()),
            },
        );
        assert_eq!(
            state.pubsub.status,
            FeedStatus::Closed {
                reason: Some("server closed the connection".into())
            }
        );
    }

    /// The token-uniqueness fix this module's doc comment describes: minting
    /// a Monitor token and a Pub/Sub token from the shared counter
    /// (`issue_feed_token`) — in either order, and even interleaved with
    /// other mints — must never produce the same value. This replaces an
    /// earlier version of this test that *manufactured* a collision by
    /// hand (setting both features' `feed_token` fields to the same value)
    /// to prove routing did not cross over on it; that scenario cannot arise
    /// at all now, so this proves the stronger, actually-relevant property:
    /// two features can never hold the same token in the first place.
    #[test]
    fn feed_tokens_are_globally_unique_across_features() {
        let mut state = State::default();
        let a = issue_feed_token(&mut state);
        let b = issue_feed_token(&mut state);
        let c = issue_feed_token(&mut state);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);

        // The shape `open_monitor_view`/Pub/Sub's own open function actually
        // use: each stores the freshly minted token on its own state.
        state.monitor.feed_token = a;
        state.pubsub.feed_token = b;
        assert_ne!(
            state.monitor.feed_token, state.pubsub.feed_token,
            "the two features must never end up holding the same token"
        );
    }

    /// With tokens globally unique, a message naming Monitor's token must
    /// touch only Monitor's status — there is no longer a *collision* to
    /// construct (the previous version of this test manufactured one by
    /// hand); this instead proves the ordinary case still respects the
    /// per-feature boundary when a second feature exists and is `Idle`.
    #[test]
    fn a_message_for_one_feature_never_touches_the_other_feature_at_all() {
        let (state, monitor_token) = connecting();
        assert_eq!(state.pubsub.status, FeedStatus::Idle);
        assert_ne!(monitor_token, state.pubsub.feed_token);

        let (state, _) = update(
            state,
            Msg::FeedOpened {
                token: monitor_token,
            },
        );
        assert_eq!(state.monitor.status, FeedStatus::Open);
        assert_eq!(
            state.pubsub.status,
            FeedStatus::Idle,
            "a message naming Monitor's token must never move Pub/Sub's status"
        );
    }

    /// Same, the other direction: a Pub/Sub token must never move Monitor.
    #[test]
    fn a_message_for_pubsub_never_touches_monitor_at_all() {
        let (state, pubsub_token) = connecting_pubsub();
        assert_eq!(state.monitor.status, FeedStatus::Idle);
        assert_ne!(pubsub_token, state.monitor.feed_token);

        let (state, _) = update(
            state,
            Msg::FeedOpened {
                token: pubsub_token,
            },
        );
        assert_eq!(state.pubsub.status, FeedStatus::Open);
        assert_eq!(state.monitor.status, FeedStatus::Idle);
    }

    /// The staleness guard, Pub/Sub side: a `FeedOpened` for a token that is
    /// no longer `state.pubsub.feed_token` must not resurrect it as `Open`.
    #[test]
    fn a_stale_pubsub_feed_opened_is_a_no_op() {
        let (state, old_token) = connecting_pubsub();
        let mut state = state;
        state.pubsub.feed_token = issue_feed_token(&mut state);
        state.pubsub.status = FeedStatus::Connecting;

        let (state, cmds) = update(state, Msg::FeedOpened { token: old_token });
        assert_eq!(
            state.pubsub.status,
            FeedStatus::Connecting,
            "a stale token must not move the current feed's status"
        );
        assert!(cmds.is_empty());
    }

    /// The staleness guard, Pub/Sub side, on the close half: an old token
    /// must not clobber a newer, already-open Pub/Sub feed.
    #[test]
    fn a_stale_pubsub_feed_closed_is_a_no_op() {
        let (state, old_token) = connecting_pubsub();
        let mut state = state;
        state.pubsub.feed_token = issue_feed_token(&mut state);
        let new_token = state.pubsub.feed_token;
        state.pubsub.status = FeedStatus::Open;

        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token: old_token,
                reason: Some("stale".into()),
            },
        );
        assert_eq!(state.pubsub.status, FeedStatus::Open);

        let (state, _) = update(
            state,
            Msg::FeedClosed {
                token: new_token,
                reason: None,
            },
        );
        assert_eq!(state.pubsub.status, FeedStatus::Closed { reason: None });
    }
}
