//! `Command` — everything the shells must do (PLAN M0.4).

use crate::key::KeyName;
use crate::mutation::Mutation;

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

/// Which feed to open (`docs/plans/m3-feed-connection.md`). A core-only
/// description with no `fred` type in it, mirroring how [`Command::ReadKey`]
/// names a key by bytes rather than by a shell-side handle.
///
/// `Subscribe` is deliberately not modelled yet — Pub/Sub's own task will
/// widen this (`Subscribe`/`Unsubscribe`/`PSubscribe` as the reader edits the
/// subscription list) without touching `Monitor`'s arm, rather than being
/// designed speculatively here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKindMsg {
    Monitor,
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
    /// Fetch type, TTL and memory usage for these rows of the Loaded set.
    ///
    /// Only ever the visible window (R2.4). Fetching metadata for a whole
    /// keyspace would be `KEYS *` with extra steps, and it would compete with
    /// `SCAN` for the connection while the list is still filling.
    FetchMetadata { indices: Vec<usize> },
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
}
