//! The Pub/Sub view's state (`g p`, R6.2, M3 task 5, `docs/plans/m3-pubsub.md`
//! phase A): subscriptions, the bounded message tail, and the feed
//! connection's own status.
//!
//! `View::PubSub` itself, the keybinding, and rendering are phase B
//! (`docs/plans/m3-pubsub.md`'s "Build order"). This module exists ahead of
//! them because the feed-connection plumbing — `Msg::PubSubMessage`,
//! `Msg::FeedOpened`/`FeedClosed` routing — needs a real
//! [`PubSubState::feed_token`]/[`PubSubState::status`] to guard against, the
//! same way Monitor's did before its own view existed
//! (`docs/plans/m3-feed-connection.md`).

use crate::command::FeedToken;
use crate::state::monitor::FeedStatus;
use crate::state::tail::LiveTail;
use std::collections::VecDeque;

/// Every message is retained up to this cap (decision 8) — the same
/// discipline as [`crate::state::monitor::MONITOR_CAP`] applied to Pub/Sub's
/// own tail, and enforced in exactly the one place, [`PubSubState::push_pubsub_message`].
pub const PUBSUB_CAP: usize = 5_000;

/// A single message's payload is truncated to this many bytes on arrival
/// (decision 8) — the same reasoning as
/// [`crate::state::monitor::MONITOR_LINE_MAX`]: a single huge `PUBLISH`
/// must not make one message bigger than the whole buffer is meant to be.
pub const PUBSUB_PAYLOAD_MAX: usize = 4 * 1024;

/// A channel literal (`SUBSCRIBE`) or a glob pattern (`PSUBSCRIBE`),
/// auto-detected from what the reader typed (decision 2): `*`, `?` or `[`
/// anywhere in the text makes it a pattern; a leading `=` forces a channel
/// (stripped from the stored name), for a channel name that would otherwise
/// look like a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subscription {
    Channel(String),
    Pattern(String),
    /// A sharded channel (`SSUBSCRIBE`, Redis 7+, M5 task 9). Delivered only
    /// by `SPUBLISH`, and only from the node that owns the channel's slot;
    /// a classic `PUBLISH` does not reach it and an `SPUBLISH` does not
    /// reach a classic [`Subscription::Channel`] of the same name. There is
    /// no sharded pattern in Redis, so there is no such variant.
    Sharded(String),
}

impl Subscription {
    /// The literal channel or pattern text, without the `=` that may have
    /// forced it.
    pub fn name(&self) -> &str {
        match self {
            Subscription::Channel(s) | Subscription::Pattern(s) | Subscription::Sharded(s) => s,
        }
    }

    pub fn is_pattern(&self) -> bool {
        matches!(self, Subscription::Pattern(_))
    }

    pub fn is_sharded(&self) -> bool {
        matches!(self, Subscription::Sharded(_))
    }
}

/// The oldest Redis that has `SSUBSCRIBE`/`SPUBLISH` (M5 task 9, decision 4).
pub const SHARDED_MIN_MAJOR: u16 = 7;

/// What the add form makes of the typed text and its sharded toggle.
///
/// With the toggle off this is [`parse_subscription`]. With it on the text is
/// always one literal sharded channel — `*`, `?` and `[` are ordinary
/// characters there, because Redis has no sharded pattern to mean them with
/// (decision 1: the form never offers pattern together with sharded). A
/// leading `=` is still stripped, so the same "force a literal" habit works
/// either way.
pub fn parse_subscription_with(input: &str, sharded: bool) -> Subscription {
    if !sharded {
        return parse_subscription(input);
    }
    Subscription::Sharded(input.strip_prefix('=').unwrap_or(input).to_string())
}

/// Auto-detect a channel or a pattern from typed input (decision 2). Never
/// fails — an empty string becomes an (unusable, but not a parse error)
/// empty channel name; the caller decides whether an empty add is even
/// offered.
pub fn parse_subscription(input: &str) -> Subscription {
    if let Some(forced) = input.strip_prefix('=') {
        return Subscription::Channel(forced.to_string());
    }
    if input.contains(['*', '?', '[']) {
        Subscription::Pattern(input.to_string())
    } else {
        Subscription::Channel(input.to_string())
    }
}

/// Redis's own glob semantics (`stringmatchlen` in `util.c`): `*` matches
/// any run (including empty), `?` matches exactly one byte, `[...]` a
/// character class (`[^...]`/`[!...]` negated, `a-z` ranges), and `\`
/// escapes the next character literally. Case-sensitive, unlike this
/// module's own display-filter glob (`crate::state::view::glob`, which is
/// intentionally a looser "filter box" match with a substring fallback and
/// no character classes) — this one has to agree with what the server
/// itself does for `PSUBSCRIBE`, not with what reads well as a UI filter.
///
/// Lives in the core, not the shell, even though its first caller
/// (`crates/app/src/redis/feed.rs`'s `matched_pattern`, attributing `via`
/// for a `pmessage`) is shell-side: [`PubSubMessage::ambiguous_via`] below
/// needs the identical matching semantics to detect the case fred's public
/// API cannot resolve on its own (two subscribed patterns matching the same
/// channel), and that detection is a pure, no-I/O, render-time question the
/// core is allowed to answer directly (ADR-0011) — one implementation,
/// tested once, used by both.
pub fn redis_glob_match(pattern: &[u8], text: &[u8]) -> bool {
    fn inner(pattern: &[u8], text: &[u8]) -> bool {
        let (mut p, mut t) = (0usize, 0usize);
        while p < pattern.len() {
            match pattern[p] {
                b'*' => {
                    // Collapse a run of consecutive `*`.
                    while p < pattern.len() && pattern[p] == b'*' {
                        p += 1;
                    }
                    if p == pattern.len() {
                        return true;
                    }
                    // `t <= text.len()` is the loop invariant, so this never
                    // underflows.
                    for skip in 0..=(text.len() - t) {
                        if inner(&pattern[p..], &text[t + skip..]) {
                            return true;
                        }
                    }
                    return false;
                }
                b'?' => {
                    if t >= text.len() {
                        return false;
                    }
                    p += 1;
                    t += 1;
                }
                b'[' => {
                    if t >= text.len() {
                        return false;
                    }
                    let (matched, next_p) = match_class(&pattern[p..], text[t]);
                    if !matched {
                        return false;
                    }
                    p += next_p;
                    t += 1;
                }
                b'\\' if p + 1 < pattern.len() => {
                    if t >= text.len() || text[t] != pattern[p + 1] {
                        return false;
                    }
                    p += 2;
                    t += 1;
                }
                c => {
                    if t >= text.len() || text[t] != c {
                        return false;
                    }
                    p += 1;
                    t += 1;
                }
            }
        }
        t == text.len()
    }

    /// `pattern` starts at `[`; returns whether `byte` is in the class and
    /// how many bytes of `pattern` the whole `[...]` class consumed.
    fn match_class(pattern: &[u8], byte: u8) -> (bool, usize) {
        let mut i = 1; // skip `[`
        let negate = pattern.get(i) == Some(&b'^') || pattern.get(i) == Some(&b'!');
        if negate {
            i += 1;
        }
        let mut found = false;
        while i < pattern.len() && pattern[i] != b']' {
            if pattern[i] == b'\\' && i + 1 < pattern.len() {
                if pattern[i + 1] == byte {
                    found = true;
                }
                i += 2;
            } else if i + 2 < pattern.len() && pattern[i + 1] == b'-' && pattern[i + 2] != b']' {
                let (lo, hi) = (
                    pattern[i].min(pattern[i + 2]),
                    pattern[i].max(pattern[i + 2]),
                );
                if (lo..=hi).contains(&byte) {
                    found = true;
                }
                i += 3;
            } else {
                if pattern[i] == byte {
                    found = true;
                }
                i += 1;
            }
        }
        let consumed = if i < pattern.len() { i + 1 } else { i }; // include the `]`
        (found != negate, consumed)
    }

    inner(pattern, text)
}

/// One message, as received from the shell's Pub/Sub read loop.
///
/// `channel` is the message's actual channel even under a pattern
/// subscription (Redis's own `pmessage` reply carries both the matched
/// pattern and the concrete channel) — `via` carries the pattern separately,
/// since "which pattern matched" and "which channel this actually was" are
/// different facts a reader may want (`m3-pubsub.md` architecture section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubSubMessage {
    /// Receipt time (the shell's injected `Clock`, ADR-0011) — Pub/Sub
    /// messages carry no server-side timestamp of their own (decision 9),
    /// unlike `MONITOR`'s lines, so this is the only time this message has.
    pub at_ms: u64,
    pub channel: Vec<u8>,
    /// The matched pattern, for a message that arrived via a `PSUBSCRIBE`d
    /// pattern (`pmessage`) rather than a direct `SUBSCRIBE` (`message`).
    pub via: Option<Vec<u8>>,
    /// Truncated to [`PUBSUB_PAYLOAD_MAX`] if it ran over — see
    /// [`PubSubState::push_pubsub_message`].
    pub payload: Vec<u8>,
    pub truncated: bool,
    /// Arrived by `SPUBLISH` on a sharded subscription (M5 task 9). Marked in
    /// the feed so a reader can tell the two apart on one channel name.
    pub sharded: bool,
}

impl PubSubMessage {
    /// Whether more than one currently-subscribed pattern could match this
    /// message's channel — the case M3 task 5 (c) names: Redis delivers one
    /// `pmessage` per matching pattern, so a channel matching two
    /// subscribed patterns arrives as two separate messages with identical
    /// channel and payload, and neither `self.via` nor anything else fred's
    /// public API exposes says which pattern actually produced *this*
    /// delivery (see `redis_glob_match`'s doc comment). When this returns
    /// `true`, the detail strip hedges its wording (`via user:* (or another
    /// matching pattern)`) rather than asserting a `via` it cannot actually
    /// back up.
    pub fn ambiguous_via(&self, subscriptions: &[Subscription]) -> bool {
        if self.via.is_none() {
            return false;
        }
        subscriptions
            .iter()
            .filter(|s| s.is_pattern() && redis_glob_match(s.name().as_bytes(), &self.channel))
            .count()
            > 1
    }
}

/// Which half of the view has keyboard focus (decision 1: `Tab` moves focus
/// between the subscription-chip strip and the tail).
///
/// The tail is the default: a reader comes back to Pub/Sub to read messages,
/// and opening onto the strip left `↑↓` doing nothing until a `Tab` they had
/// no reason to know about. The strip is where you go to edit the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PubSubFocus {
    Strip,
    #[default]
    Tail,
}

/// The Pub/Sub view's whole state (`View::PubSub`, phase B).
///
/// The buffer/pause/following/selection/filter discipline lives in
/// [`LiveTail`] (`docs/plans/m3-pubsub.md` decision 10) — `PubSubState`
/// wraps one and `Deref`/`DerefMut`s to it, the same shape
/// [`crate::state::monitor::MonitorState`] uses, so `state.pubsub.paused`,
/// `.filter`, `.following` and the movement/pause methods read and write
/// directly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PubSubState {
    tail: LiveTail<PubSubMessage>,
    /// Remembered for the session (decision 3): never persisted, forgotten
    /// on quit, but not cleared just because the view closed — leaving the
    /// view drops the connection, `g p` resubscribes to this same list.
    pub subscriptions: Vec<Subscription>,
    /// Where the feed connection stands (`docs/plans/m3-feed-connection.md`).
    pub status: FeedStatus,
    /// Identifies the most recently issued `Command::OpenFeed` for this
    /// view — the same discipline as
    /// [`crate::state::monitor::MonitorState::feed_token`]. Minted from the
    /// one shared counter every `FeedToken` comes from
    /// (`state.feed_token_seq`, `update::issue_feed_token`), so this can
    /// never equal `state.monitor.feed_token` at the same time — a
    /// `Msg::FeedOpened`/`Msg::FeedClosed` routes to exactly one feature by
    /// construction, not because of an ordering invariant about which
    /// feature's `status` happens to be `Idle` when (`crate::update::feed`).
    pub feed_token: FeedToken,
    pub focus: PubSubFocus,
    /// Which chip is selected on the subscription strip (decision 1: `←→`
    /// picks a chip, `d` unsubscribes it). Meaningless — and left at
    /// whatever it was — while `subscriptions` is empty or `focus` is
    /// `Tail`.
    pub selected_chip: usize,
    /// The add-input's typed text (`a add`, decision 1), parsed by
    /// [`parse_subscription`] on submit.
    pub input: String,
    /// Whether the add-input is capturing text right now (`a`, or opened
    /// automatically when the view is entered with no remembered
    /// subscription — decision 6). A distinct capture mode from
    /// `State::filtering` (`/`, scoped to the tail): the two never overlap,
    /// since one lives on the strip and the other on the tail, but they are
    /// different actions on different keys and must not be conflated into
    /// one bool.
    pub adding: bool,
    /// The add form's sharded toggle (M5 task 9, decision 1) — a field of the
    /// form, not a key of the view. Only meaningful while `adding`; cleared
    /// whenever the form opens or closes.
    pub sharded: bool,
}

impl std::ops::Deref for PubSubState {
    type Target = LiveTail<PubSubMessage>;
    fn deref(&self) -> &Self::Target {
        &self.tail
    }
}

impl std::ops::DerefMut for PubSubState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.tail
    }
}

impl PubSubState {
    pub fn messages(&self) -> &VecDeque<PubSubMessage> {
        self.tail.items()
    }

    pub fn selected_message(&self) -> Option<&PubSubMessage> {
        self.tail.selected_item()
    }

    /// Start a fresh view (`g p`, `update::pubsub::open_pubsub`): resets the
    /// tail, focus, chip selection and add-input, but — decision 3's whole
    /// point — leaves `subscriptions` untouched. `status`/`feed_token` are
    /// the caller's job (`open_pubsub` decides whether to dial immediately
    /// or stay `Idle` for the lazy-open case, decision 6), not this
    /// method's, since that decision needs `&mut State` to mint a token.
    pub fn open_fresh(&mut self) {
        self.tail.reset();
        self.tail.following = true;
        self.focus = PubSubFocus::default();
        self.selected_chip = 0;
        self.input.clear();
        self.adding = false;
        self.sharded = false;
    }

    /// The one function that appends to the tail (CLAUDE.md: "the cap is
    /// enforced in exactly one place"), mirroring
    /// [`crate::state::monitor::MonitorState::push_monitor_line`].
    /// Truncates the payload to [`PUBSUB_PAYLOAD_MAX`] and delegates the
    /// cap/pause discipline to [`LiveTail::push`] with [`PUBSUB_CAP`].
    pub fn push_pubsub_message(
        &mut self,
        at_ms: u64,
        channel: Vec<u8>,
        via: Option<Vec<u8>>,
        payload: Vec<u8>,
    ) {
        self.push_message(at_ms, channel, via, payload, false);
    }

    /// [`Self::push_pubsub_message`] with the message's kind: `sharded` for an
    /// `SMESSAGE`. Same single enforcement point for the cap.
    pub fn push_message(
        &mut self,
        at_ms: u64,
        channel: Vec<u8>,
        via: Option<Vec<u8>>,
        mut payload: Vec<u8>,
        sharded: bool,
    ) {
        let truncated = payload.len() > PUBSUB_PAYLOAD_MAX;
        if truncated {
            // Raw bytes, not a `String` — no char-boundary concern the way
            // `MonitorLine`'s truncation has.
            payload.truncate(PUBSUB_PAYLOAD_MAX);
        }
        self.tail.push(
            PubSubMessage {
                at_ms,
                channel,
                via,
                payload,
                truncated,
                sharded,
            },
            PUBSUB_CAP,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_a_channel() {
        assert_eq!(
            parse_subscription("orders"),
            Subscription::Channel("orders".into())
        );
    }

    #[test]
    fn a_glob_star_is_a_pattern() {
        assert_eq!(
            parse_subscription("user:*"),
            Subscription::Pattern("user:*".into())
        );
    }

    #[test]
    fn a_question_mark_is_a_pattern() {
        assert_eq!(
            parse_subscription("k?y"),
            Subscription::Pattern("k?y".into())
        );
    }

    #[test]
    fn a_bracket_class_is_a_pattern() {
        assert_eq!(
            parse_subscription("k[ae]y"),
            Subscription::Pattern("k[ae]y".into())
        );
    }

    #[test]
    fn a_leading_equals_forces_a_channel_and_is_stripped() {
        assert_eq!(
            parse_subscription("=user:*"),
            Subscription::Channel("user:*".into())
        );
    }

    #[test]
    fn the_cap_is_never_exceeded_over_a_long_synthetic_run() {
        let mut p = PubSubState::default();
        for i in 0..(PUBSUB_CAP * 2) {
            p.push_pubsub_message(i as u64, b"c".to_vec(), None, format!("m{i}").into_bytes());
            assert!(p.messages().len() <= PUBSUB_CAP, "grew past the cap at {i}");
        }
        assert_eq!(p.messages().len(), PUBSUB_CAP);
    }

    #[test]
    fn a_payload_over_the_byte_cap_is_truncated_and_marked() {
        let mut p = PubSubState::default();
        let huge = vec![b'x'; PUBSUB_PAYLOAD_MAX * 3];
        p.push_pubsub_message(1, b"c".to_vec(), None, huge);
        let msg = p.messages().back().unwrap();
        assert_eq!(msg.payload.len(), PUBSUB_PAYLOAD_MAX);
        assert!(msg.truncated);
    }

    #[test]
    fn a_short_payload_is_not_marked_truncated() {
        let mut p = PubSubState::default();
        p.push_pubsub_message(1, b"c".to_vec(), None, b"short".to_vec());
        assert!(!p.messages().back().unwrap().truncated);
    }

    #[test]
    fn pausing_counts_messages_rather_than_buffering_them() {
        let mut p = PubSubState::default();
        p.push_pubsub_message(1, b"c".to_vec(), None, b"one".to_vec());
        p.toggle_pause();
        assert!(p.paused);
        for i in 0..10 {
            p.push_pubsub_message(i, b"c".to_vec(), None, b"dropped".to_vec());
        }
        assert_eq!(p.messages().len(), 1, "the buffer never grew while paused");
        assert_eq!(p.dropped_while_paused, 10);
    }

    #[test]
    fn a_pattern_message_carries_both_the_matched_pattern_and_the_concrete_channel() {
        let mut p = PubSubState::default();
        p.push_pubsub_message(
            1,
            b"user:42".to_vec(),
            Some(b"user:*".to_vec()),
            b"hi".to_vec(),
        );
        let msg = p.messages().back().unwrap();
        assert_eq!(msg.channel, b"user:42");
        assert_eq!(msg.via.as_deref(), Some(b"user:*".as_slice()));
    }

    #[test]
    fn a_direct_channel_message_carries_no_via() {
        let mut p = PubSubState::default();
        p.push_pubsub_message(1, b"orders".to_vec(), None, b"hi".to_vec());
        assert_eq!(p.messages().back().unwrap().via, None);
    }

    #[test]
    fn glob_star_matches_any_run_including_empty() {
        assert!(redis_glob_match(b"user:*", b"user:42"));
        assert!(redis_glob_match(b"user:*", b"user:"));
        assert!(!redis_glob_match(b"user:*", b"order:42"));
    }

    #[test]
    fn glob_question_mark_matches_exactly_one_byte() {
        assert!(redis_glob_match(b"k?y", b"key"));
        assert!(!redis_glob_match(b"k?y", b"ky"));
        assert!(!redis_glob_match(b"k?y", b"keey"));
    }

    #[test]
    fn glob_bracket_class_matches_a_set_or_range() {
        assert!(redis_glob_match(b"k[ae]y", b"kay"));
        assert!(redis_glob_match(b"k[ae]y", b"key"));
        assert!(!redis_glob_match(b"k[ae]y", b"kiy"));
        assert!(redis_glob_match(b"k[a-c]y", b"kby"));
        assert!(!redis_glob_match(b"k[a-c]y", b"kdy"));
    }

    #[test]
    fn glob_negated_bracket_class_excludes_the_set() {
        assert!(redis_glob_match(b"k[^a]y", b"kby"));
        assert!(!redis_glob_match(b"k[^a]y", b"kay"));
    }

    #[test]
    fn glob_with_no_wildcards_is_an_exact_match_not_a_substring() {
        assert!(redis_glob_match(b"orders", b"orders"));
        assert!(!redis_glob_match(b"orders", b"my-orders"));
    }

    #[test]
    fn ambiguous_via_is_false_for_a_direct_channel_message() {
        let msg = PubSubMessage {
            at_ms: 1,
            channel: b"orders".to_vec(),
            via: None,
            payload: b"hi".to_vec(),
            truncated: false,
            sharded: false,
        };
        let subs = vec![
            Subscription::Pattern("user:*".into()),
            Subscription::Pattern("user:4?".into()),
        ];
        assert!(!msg.ambiguous_via(&subs));
    }

    #[test]
    fn ambiguous_via_is_false_with_only_one_matching_subscribed_pattern() {
        let msg = PubSubMessage {
            at_ms: 1,
            channel: b"user:42".to_vec(),
            via: Some(b"user:*".to_vec()),
            payload: b"hi".to_vec(),
            truncated: false,
            sharded: false,
        };
        let subs = vec![
            Subscription::Pattern("user:*".into()),
            Subscription::Pattern("order:*".into()),
        ];
        assert!(!msg.ambiguous_via(&subs));
    }

    #[test]
    fn ambiguous_via_is_true_when_two_subscribed_patterns_both_match() {
        let msg = PubSubMessage {
            at_ms: 1,
            channel: b"user:42".to_vec(),
            via: Some(b"user:*".to_vec()),
            payload: b"hi".to_vec(),
            truncated: false,
            sharded: false,
        };
        let subs = vec![
            Subscription::Pattern("user:*".into()),
            Subscription::Pattern("user:4?".into()),
        ];
        assert!(msg.ambiguous_via(&subs));
    }
}
