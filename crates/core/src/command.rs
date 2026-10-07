//! `Command` — everything the shells must do (PLAN M0.4).

use crate::key::KeyName;
use crate::mutation::Mutation;
use crate::state::pubsub::Subscription;

/// Which read a reply belongs to.
///
/// Reads are asynchronous and there can be more than one in flight, so a reply
/// has to say *which question it answers*. Without that, the last reply to
/// arrive wins regardless of what the user asked for last: opening a slow key
/// and then a fast one left the slow one's reply to land second and replace the
/// Open key — the value pane showing a key the user was not on.
///
/// A name would not do the job. Open A, open B, open A again, and the first
/// A's reply matches by name while answering a question two reads out of date.
/// Only an identity that changes on *every* read is sufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReadToken(u64);

impl ReadToken {
    /// The identity for the next read. Crate-private: only
    /// [`crate::update::update`] mints tokens, so a shell cannot invent one and
    /// resurrect a superseded read. A shell only ever holds tokens it was handed
    /// in a [`Command::ReadKey`] (review H2).
    pub(crate) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Which feed connection a [`crate::Msg::FeedOpened`]/[`crate::Msg::FeedClosed`]
/// reports on (`docs/plans/m3-feed-connection.md`).
///
/// Mirrors [`ReadToken`] for the same reason: a feed is opened and closed by
/// the reader entering and leaving a view (`g m`, `Esc`), but the shell's read
/// loop only discovers the server side of a close asynchronously. A message
/// about a feed the reader has already replaced — `Esc` then `g m` again
/// before the old feed's teardown was reported — must not be mistaken for one
/// about the feed currently open. Only an identity that changes on every
/// [`Command::OpenFeed`] tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeedToken(u64);

impl FeedToken {
    /// The identity for the next feed. Crate-private, for the same reason as
    /// [`ReadToken::next`]: only [`crate::update::update`] mints one — here,
    /// `g m` (`docs/plans/m3-monitor.md`, phase B).
    pub(crate) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Which `Command::FetchServerInfo`/`Msg::ServerInfoLoaded`/
/// `Msg::ServerInfoFailed` reply belongs to (M3 task 6,
/// `docs/plans/m3-dashboard.md`).
///
/// Mirrors [`ReadToken`] for exactly the same reason, one poll wide: a
/// manual `g d`/`r` can overlap a timer-driven poll
/// (`Msg::DashboardPollTick`), and without an identity a slow reply that
/// lands *second* could overwrite a newer one that already landed — which
/// would also corrupt the previous-poll comparison the rising-counter
/// alarms rely on (`DashboardState::record_poll`'s `previous` rotation).
/// Minted only by [`crate::update::update`], never by a shell, for the same
/// reason [`ReadToken::next`] is crate-private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InfoToken(u64);

impl InfoToken {
    pub(crate) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Which numbering of the Loaded set a metadata fetch was issued against
/// (M4 task 4, decision 6, `docs/plans/m4-perf-interaction.md`).
///
/// A [`crate::Msg::MetadataBatch`] names rows by Loaded set index, and a
/// rescan renumbers them (`LoadedSet::clear`, then a refill in a different
/// `SCAN` order). The core bumps this exactly there and drops any reply whose
/// epoch is not the current one, so a late reply can never write a type, TTL,
/// size or tombstone onto an unrelated key. Minted only by
/// [`crate::update::update`], never by a shell — the same shape and
/// discipline as [`InfoToken`] and [`ReadToken`]: the shell echoes it back
/// unchanged, and the core alone decides what is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MetadataEpoch(u64);

impl MetadataEpoch {
    pub(crate) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// Which feed to open (`docs/plans/m3-feed-connection.md`). A core-only
/// description with no `fred` type in it, mirroring how [`Command::ReadKey`]
/// names a key by bytes rather than by a shell-side handle.
///
/// `Subscribe` carries the subscription list to dial with (`m3-pubsub.md`
/// phase A) — unlike `Monitor`, which needs nothing beyond the feed itself,
/// a Pub/Sub connection has to know what to `SUBSCRIBE`/`PSUBSCRIBE` to
/// before it is worth opening at all (and phase A's lazy-open decision 6:
/// `g p` with an empty list dials nothing until the first subscription, so
/// this variant is only ever reached with a non-empty list in practice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedKindMsg {
    Monitor,
    Subscribe(Vec<Subscription>),
}

/// Work the core cannot perform itself. A shell executes these and reports back
/// as a [`crate::Msg`].
///
/// Every Redis read belongs here, and there is exactly one variant for it, so
/// the arming step that liveness depends on cannot be forgotten on one branch
/// of several (ADR-0006).
/// Deliberately **not** `#[non_exhaustive]`. That attribute exists to let a
/// crate add variants without breaking downstream matches — which is precisely
/// the opposite of what is wanted here. Every `Command` must be executed by a
/// shell, so adding one should fail the shell's `match` at compile time rather
/// than silently doing nothing at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Tear down the terminal and exit.
    Quit,
    /// Read a key from the server: opening it, or refetching the one already
    /// open. The one read command (ADR-0006).
    ///
    /// `key` is the exact bytes the server knows the key by, carried here
    /// rather than looked up by the shell from the Open key. The Open key's
    /// name used to be display text, so a key that was not valid UTF-8
    /// refetched a *different* key, came back `✕ deleted`, and armed tracking
    /// on that other key (review C1).
    ///
    /// `arm` says whether this read sends `CLIENT CACHING YES` with it, and the
    /// core decides it — from the same [`crate::state::Link`] the header's
    /// liveness is derived from — so the shell holds no second copy of the
    /// tracking capability to fall out of step with it (review H3). Tracking
    /// is consumed by the invalidation it produces (verified against Redis
    /// 8.4.0), so every read on a tracking connection arms; there is no way to
    /// issue one that does not.
    ReadKey {
        key: KeyName,
        /// The Loaded set row this key was read from, if one is known.
        index: Option<usize>,
        token: ReadToken,
        arm: bool,
    },
    /// Try to connect again after the given delay.
    Reconnect { after_ms: u64 },
    /// Begin traversing the keyspace. `SCAN` only, never `KEYS` — cursor-based,
    /// streaming and resumable (R2.1).
    StartScan { pattern: Option<String> },
    /// Stop an in-flight traversal. Every long operation is cancellable.
    CancelScan,
    /// (Re)start the filter debounce timer (M4 task 4, decision 2): the typed
    /// filter is ahead of the list and the shell should answer with
    /// [`crate::Msg::FilterRebuildDue`] once it has been quiet for the
    /// window. Issued on every keystroke that cannot be narrowed, so a burst
    /// coalesces into one rebuild. The core owns no clock; this is the only
    /// thing it asks of one.
    ScheduleFilterRebuild,
    /// A rebuild job is running and wants its next slice: the shell answers
    /// with [`crate::Msg::RebuildStep`] once the frame has been drawn and any
    /// input already waiting has been handled (M6 task 3). Appended by
    /// `update` to every message's commands while a job runs, so the shell
    /// never has to remember whether one is.
    ContinueRebuild,
    /// Fetch type, TTL and memory usage for these rows of the Loaded set.
    ///
    /// Only ever the visible window (R2.4). Fetching metadata for a whole
    /// keyspace would be `KEYS *` with extra steps, and it would compete with
    /// `SCAN` for the connection while the list is still filling.
    ///
    /// `epoch` is the Loaded set numbering `indices` belong to; the shell
    /// echoes it in the reply and the core drops a reply from any other.
    FetchMetadata {
        indices: Vec<usize>,
        epoch: MetadataEpoch,
    },
    /// Execute a confirmed write (R4.4).
    ///
    /// Only ever issued by confirming a [`crate::state::PendingMutation`]: the
    /// one point where the chokepoint's decision (allowed, or refused by
    /// Read-only Mode) becomes work a shell will actually do. The shell answers
    /// with [`crate::Msg::MutationSettled`], never with a value read off the
    /// write's own reply — the core's only path for a value to reach the Viewer
    /// is a real read (ADR-0006).
    ///
    /// One variant for every write (review H1): adding a mutation adds a
    /// [`Mutation`] variant, not a command, a shell arm and a reply message.
    Execute {
        mutation: Mutation,
        /// The Loaded set row a `DeleteKey` was staged from, echoed back in
        /// the reply so the tombstone lands on it. `None` for every other write.
        index: Option<usize>,
    },
    /// Put text on the clipboard.
    ///
    /// The core builds the text; how it reaches a clipboard is the shell's
    /// problem, and over SSH a harder one than it appears.
    CopyToClipboard { text: String, label: String },
    /// Raise a notice, dated by the shell's clock.
    ///
    /// The core has no clock of its own (ADR-0011), so it cannot date a notice
    /// it raises. This carries the words; the shell supplies the moment.
    Notify { text: String },
    /// Fetch the server's slowlog ring buffer (R6.4, M3,
    /// `docs/plans/m3-slowlog.md`): `SLOWLOG GET <count>`. Issued by `g s`
    /// and by `r` while the Slowlog view is showing — there is no `CLIENT
    /// TRACKING` equivalent for it, so unlike [`Command::ReadKey`] this
    /// carries no arming and answers with [`crate::Msg::SlowlogLoaded`]
    /// rather than a push.
    FetchSlowlog { count: i64 },
    /// Fetch the server's own vitals (R6.3, M3 task 6,
    /// `docs/plans/m3-dashboard.md`): `INFO`. Issued by `g d`, by `r`, and by
    /// [`crate::update::update`] reacting to a `Msg::DashboardPollTick` while
    /// the Dashboard is on screen (the shell's interval always ticks —
    /// decision 1, the core never owns the timer itself — but only a tick
    /// the core actually acts on produces this). Like
    /// [`Command::FetchSlowlog`] this is a plain request/response on the
    /// main connection, not a feed — `INFO` has no `CLIENT TRACKING`
    /// equivalent, so there is nothing to (re-)arm. `token` is
    /// [`InfoToken`]'s own reason to exist: a manual fetch and a poll can
    /// overlap, and only the reply carrying the *current* token is allowed
    /// to land.
    FetchServerInfo { token: InfoToken },
    /// Stop talking to the Cluster's nodes one by one: the Dashboard or the
    /// Slowlog was left (M5 tasks 7 and 8). The shell aborts a poll or fetch
    /// still in flight and closes the per-node connections they opened. Only
    /// ever emitted on a Cluster, the one place anything holds them.
    CloseNodeConnections,
    /// Open a second, dedicated connection to the same resolved target and
    /// start streaming from it (`docs/plans/m3-feed-connection.md`) — `MONITOR`
    /// today, Pub/Sub later widens [`FeedKindMsg`]. The render loop never does
    /// I/O (CLAUDE.md); this only ever asks a shell to.
    ///
    /// `token` is minted the same way [`Command::ReadKey`]'s is: it names
    /// *this* feed, so a shell reply about a feed the reader has already left
    /// (`Esc`, `g m` again) can be told apart from one about the feed
    /// currently open.
    OpenFeed { kind: FeedKindMsg, token: FeedToken },
    /// Tear down whatever feed connection is open: cancel its token, drop its
    /// handle — [`Command::CancelScan`]'s sibling for the second connection
    /// (`docs/plans/m3-feed-connection.md`). A no-op if none is open.
    CloseFeed,
    /// Add and/or remove subscriptions on the **already-open** Pub/Sub feed
    /// (`docs/plans/m3-pubsub.md` phase A) — `SUBSCRIBE`/`PSUBSCRIBE` for
    /// `add`, `UNSUBSCRIBE`/`PUNSUBSCRIBE` for `remove`, issued on the same
    /// connection [`Command::OpenFeed`] with
    /// [`FeedKindMsg::Subscribe`] dialed. Deliberately not a
    /// close-then-reopen: tearing the feed down to add one channel would
    /// drop in-flight messages on every channel already subscribed, which is
    /// the one thing a reader adding a second subscription mid-session must
    /// never see happen to the first.
    UpdateSubscription {
        add: Vec<Subscription>,
        remove: Vec<Subscription>,
    },
}
