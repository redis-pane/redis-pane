//! `Msg` — everything that can happen (PLAN M0.4).

/// Metadata for one key, as fetched.
///
/// `ttl_seconds` uses the store's convention: `-1` means the key has no expiry,
/// which is a fact about the key rather than a gap in what we know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataEntry {
    pub index: usize,
    pub kind: crate::state::KeyKind,
    pub ttl_seconds: i32,
    pub size_bytes: u32,
}

/// A key, described without reference to any terminal library.
///
/// The shell translates crossterm's events into these. That translation is the
/// boundary doing its job: `crossterm` is not reachable from the core, so the
/// core cannot accidentally grow a dependency on how input arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    /// A function key, `F1`–`F(255)`. Added for `F1` (contextual help, M3):
    /// a global alias for `Action::Help` that works from every mode, since
    /// `?` is a typed character or is swallowed everywhere but Normal mode.
    F(u8),
}

/// A keypress with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyPress {
    /// An unmodified key.
    pub fn plain(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
        }
    }

    /// A key held with Control.
    pub fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: true,
            alt: false,
        }
    }

    /// Whether this is the character `c` with no modifiers.
    pub fn is_char(&self, c: char) -> bool {
        self.code == KeyCode::Char(c) && !self.ctrl && !self.alt
    }
}

/// A mouse action, described without reference to any terminal library — the
/// same boundary `KeyCode`/`KeyPress` draw for the keyboard (R7.3).
///
/// Deliberately not "everything a mouse can do": only the left button is
/// modelled, because nothing here is specified for the others, and a
/// right-click doing something the reader did not ask for is worse than a
/// right-click doing nothing. `col`/`row` are cell coordinates in the whole
/// terminal, the same space [`crate::render::layout::layout`] lays panes out
/// in — the core, not the shell, decides what a coordinate means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseAction {
    /// The left button went down at this cell.
    Down { col: u16, row: u16 },
    /// The left button was released, wherever it happened to be.
    Up,
    /// The cursor moved to this cell while the left button was held.
    Drag { col: u16, row: u16 },
    /// The wheel scrolled one notch toward the top, over this cell.
    ScrollUp { col: u16, row: u16 },
    /// The wheel scrolled one notch toward the bottom, over this cell.
    ScrollDown { col: u16, row: u16 },
}

/// Every input to the core: keystrokes, resizes, and replies from the shells.
///
/// The core has no other way in. A shell that wants to tell the core something
/// adds a variant here rather than reaching into state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Msg {
    /// A key was pressed.
    Key(KeyPress),
    /// The mouse did something (R7.3).
    Mouse(MouseAction),
    /// The terminal was resized. Drives the layout breakpoints in DESIGN §2.
    Resized {
        cols: u16,
        rows: u16,
    },
    /// A read of the open key completed at this clock reading.
    ReadCompleted {
        at_ms: u64,
    },
    /// A read was just dispatched to the shell, at this clock reading.
    ///
    /// Exists only to timestamp `PendingRead` (`crate::state::PendingRead`)
    /// for the loading indicator's delay gate — `update()` has no clock of
    /// its own (ADR-0011), so the shell, which actually dispatches the read,
    /// supplies the one fact the core needs. A token that no longer names the
    /// outstanding read (superseded, or already answered) is ignored.
    ReadIssued {
        token: crate::command::ReadToken,
        at_ms: u64,
    },
    /// The shell connected. `tracking_supported` is the result of *attempting*
    /// `CLIENT TRACKING`, never an inference from the version (ADR-0007).
    Connected {
        version: String,
        tracking_supported: bool,
    },
    /// The link dropped mid-session.
    ConnectionLost,
    /// A reconnect attempt is scheduled. Backoff is visible, never a silent wait.
    ReconnectScheduled {
        attempt: u32,
        retry_in_ms: u64,
    },
    /// The server accepted `CLIENT CACHING YES` for the open key. Only this
    /// message can make the header say `live`.
    TrackingArmed,
    /// An invalidation push arrived for the open key. It consumed the arming.
    Invalidated,
    /// A traversal began. `estimated_total` is `DBSIZE` at that moment.
    ScanStarted {
        estimated_total: u64,
    },
    /// A page of keys arrived. The core never sees the cursor that produced it,
    /// so one cursor can become N without the browser noticing (ADR-0008).
    ScanBatch {
        keys: Vec<Vec<u8>>,
    },
    /// The traversal finished on its own.
    ScanComplete,
    /// The traversal stopped early because the shell was asked to stop.
    ScanCancelled,
    ScanFailed {
        error: String,
    },
    /// Lazily-fetched metadata arrived for some rows (R2.4).
    ///
    /// `gone` carries the rows whose key had vanished by the time the fetch
    /// reached it. They are indices rather than `MetadataEntry` values with a
    /// flag: every field of a `MetadataEntry` describes metadata, and a key that
    /// is not there has none, so a flagged entry would have to invent a type.
    MetadataBatch {
        entries: Vec<MetadataEntry>,
        gone: Vec<usize>,
        /// When this batch was read, so the keys pane's TTL column can count
        /// down locally afterward (R3.9's rule, extended from the Viewer to
        /// here) instead of freezing at whatever it read until the next
        /// rescan.
        at_ms: u64,
        /// The epoch of the `Command::FetchMetadata` this answers. A batch
        /// from before the last rescan is dropped unread: its indices name
        /// different keys now.
        epoch: crate::command::MetadataEpoch,
    },
    /// A read of the open key completed, carrying what the server said.
    ///
    /// The only way a value enters the Viewer. There is no other path, which is
    /// what makes a stale value unrepresentable (ADR-0006).
    ValueLoaded {
        /// Which read this answers. A reply from a superseded read is dropped
        /// rather than applied — see [`crate::command::ReadToken`].
        token: crate::command::ReadToken,
        /// The Loaded set row this key was read from, if one was known. `None`
        /// after a rescan took it away — the value is still the value, but
        /// there is no row it may be written back to.
        index: Option<usize>,
        name: crate::key::KeyName,
        value: crate::state::Value,
        ttl_seconds: i32,
        size_bytes: u32,
        at_ms: u64,
    },
    /// The key that was read is gone: deleted, expired, or evicted — or was
    /// never there to begin with.
    ///
    /// Carries the same identity `ValueLoaded` does, and for the same reason:
    /// the token alone only says *this is the most recent read*, not *which
    /// key it was a read of*. A token-only version of this message shipped
    /// once and had exactly the bug that identity exists to prevent —
    /// opening a gone key while a different one was already open badged the
    /// *previous*, perfectly alive key `✕ deleted` and tombstoned its row,
    /// because the handler had no way to tell the two apart and simply
    /// mutated whatever was open. `index`/`name` are what let the core ask
    /// "is this the key I already have open, or a different one" instead of
    /// assuming.
    ValueGone {
        token: crate::command::ReadToken,
        /// The Loaded set row this key was read from, if one was known —
        /// same meaning as [`Msg::ValueLoaded::index`].
        index: Option<usize>,
        name: crate::key::KeyName,
        at_ms: u64,
    },
    /// Something was copied. Drives a notice that fades on its own.
    ///
    /// The label is owned rather than `&'static str` because a copy that could
    /// only take part of a value has to say so, and how much it took is not
    /// known until the copy is built.
    Copied {
        label: String,
        at_ms: u64,
    },
    /// The core raised a notice and needs the shell's clock to date it.
    ///
    /// `update` is pure and has no clock (ADR-0011), so a notice it raises by
    /// itself cannot be timestamped where it is written. Two of them were built
    /// with `at_ms: 0` and were therefore invisible for the life of the
    /// process: `notice_now` shows a notice for 2.5 seconds, and the shell's
    /// clock reads epoch milliseconds. The message existed, explained itself,
    /// and could never appear. Round-tripping through the shell is how every
    /// other dated fact reaches the core, and it is how these do now.
    Noticed {
        text: String,
        at_ms: u64,
    },
    /// Conditions the server reported: replica status, and anything currently
    /// rejecting writes (R1.15). Sent at connect and on every reconnect,
    /// because a failover can change the answer.
    ServerState {
        read_only: Option<crate::state::ReadOnlyReason>,
        condition: Option<crate::state::ServerCondition>,
    },
    /// The terminal reported a bracketed paste (ADR-0014).
    ///
    /// With the inline editor open this is one `insert_str`, staged as a
    /// single undo step; while the filter is capturing it is appended
    /// (newlines stripped); otherwise it is ignored.
    Paste(String),
    /// A [`crate::Command::Execute`] finished (R4.1, R4.3, R7.4).
    ///
    /// Carries the mutation it answers, so the core — not the shell — decides
    /// what each outcome means: a delete tombstones, a write refetches, a guard
    /// that refused hands the edit back, and an error is shown with the command
    /// that failed (review H1). `result`'s `Err` is the server's error detail.
    /// Guarded by the mutation's key: the reader may have moved on to a
    /// different key by the time this lands.
    MutationSettled {
        mutation: crate::mutation::Mutation,
        /// As issued in [`crate::Command::Execute`].
        index: Option<usize>,
        result: Result<crate::mutation::MutationOutcome, String>,
        at_ms: u64,
    },
    /// An operation failed. Carries the command that failed (R7.4).
    ///
    /// Errors are never swallowed: a Redis error that produces no visible
    /// effect is indistinguishable from the app deciding to do nothing, which
    /// is the class of defect this project exists to remove.
    Failed {
        command: String,
        detail: String,
        at_ms: u64,
    },
    /// The user asked to leave.
    Quit,
    /// `Command::FetchSlowlog` answered (R6.4, M3).
    SlowlogLoaded {
        entries: Vec<crate::state::SlowlogEntry>,
    },
    /// `Command::FetchSlowlog` failed. Produces both the R7.4 notification
    /// naming the failing command and the Slowlog view's own in-screen empty
    /// state — two readers of the same fact (`update::slowlog::slowlog_failed`).
    SlowlogFailed {
        detail: String,
        at_ms: u64,
    },
    /// `Command::FetchServerInfo` answered (R6.3, M3 task 6,
    /// `docs/plans/m3-dashboard.md`): the shell's parse of `INFO` into a
    /// core-owned [`crate::state::RawInfo`] — no `fred` types cross this
    /// boundary (ADR-0011). `at_ms` is the shell's injected clock reading at
    /// receipt, carried through rather than re-derived, so `DashboardState`'s
    /// "updated Ns ago" readout is a function of state alone (ADR-0011).
    /// `token` is the one the `FetchServerInfo` that started it carried; a
    /// reply whose token is not the current poll's is stale (a manual `r`
    /// that overlapped a timer poll, or vice versa) and is dropped rather
    /// than overwriting a newer answer — see [`crate::command::InfoToken`].
    ServerInfoLoaded {
        info: crate::state::RawInfo,
        at_ms: u64,
        token: crate::command::InfoToken,
    },
    /// `Command::FetchServerInfo` failed. Produces both the R7.4 notification
    /// naming the failing command and the Dashboard's own in-view error
    /// state — the same shape [`Msg::SlowlogFailed`] already has. The last
    /// good values stay on screen with their age; nothing goes blank
    /// (decision 7, `docs/plans/m3-dashboard.md`). Guarded by `token` exactly
    /// like [`Msg::ServerInfoLoaded`].
    ServerInfoFailed {
        detail: String,
        at_ms: u64,
        token: crate::command::InfoToken,
    },
    /// One tick of the Dashboard's poll interval (decision 1,
    /// `docs/plans/m3-dashboard.md`): the shell's `tokio::time::interval`
    /// always ticks (so it never accrues missed-tick catch-up bursts — see
    /// `terminal::run`'s own comment), but only ever produces this `Msg`,
    /// leaving [`crate::update::update`] to decide whether the tick actually
    /// means anything — issuing `Command::FetchServerInfo` only while
    /// `View::Dashboard` is on screen, the main connection is up, and no
    /// poll is already in flight (`DashboardState::loading`). This is what
    /// keeps the timer itself a shell concern while the *decision* stays in
    /// the core, testable without a real clock or a real interval.
    DashboardPollTick,
    /// The shell's filter debounce window closed (M4 task 4, decision 2): if
    /// the typed filter is still ahead of the list, rebuild it now. The core
    /// decides whether anything is owed ([`crate::State::filter_pending`]);
    /// a stale or duplicate one is a no-op.
    FilterRebuildDue,
    /// A feed connection (`Command::OpenFeed`) finished dialing and is
    /// streaming (`docs/plans/m3-feed-connection.md`). `token` is the one the
    /// `OpenFeed` that started it carried; a token that does not name the feed
    /// currently open is stale and ignored (a reader who left the view, or
    /// opened a newer feed, before this landed).
    FeedOpened {
        token: crate::command::FeedToken,
    },
    /// A feed connection ended: closed by the reader (`Command::CloseFeed`),
    /// by the server, or by the network going silent. `reason` is `None` for a
    /// close the reader asked for, `Some(_)` for anything they did not
    /// (`m3-monitor.md` decision 9: the view keeps its buffer, the header
    /// says why, and `r` reopens — no automatic reconnect). Guarded by `token`
    /// exactly like [`Msg::FeedOpened`].
    FeedClosed {
        token: crate::command::FeedToken,
        reason: Option<String>,
    },
    /// One line from an open `MONITOR` feed, translated by the shell's read
    /// loop. `at_ms` is receipt time from the injected `Clock` (ADR-0011):
    /// `MONITOR`'s own server-side timestamp is carried in `raw`, parsed for
    /// display by `MonitorLine::columns` (`m3-monitor.md` decision 8), but
    /// local buffer ordering stays a pure function of injected state, never
    /// of the server's clock. `token` guards against a line from a feed the
    /// reader has already left arriving after a newer `g m` reset the tail —
    /// the same discipline `Msg::FeedOpened`/`Msg::FeedClosed` follow, and
    /// necessary here for the same reason: `MonitorLine` carries no other
    /// identity a stale line could be told apart by.
    MonitorLine {
        token: crate::command::FeedToken,
        at_ms: u64,
        raw: String,
    },
    /// One message from an open Pub/Sub feed (`docs/plans/m3-pubsub.md`
    /// phase A), translated by the shell's read loop — a `message` reply
    /// (`via: None`) or a `pmessage` reply (`via: Some(pattern)`). `at_ms`
    /// is receipt time from the injected `Clock`, the only timestamp a
    /// Pub/Sub message has (decision 9: unlike `MONITOR`, Redis attaches no
    /// server-side time to a published message). Guarded by `token` exactly
    /// like [`Msg::MonitorLine`] — this variant needs no cross-feature
    /// ambiguity guard of its own, since it names Pub/Sub by construction;
    /// only the feed-connection lifecycle messages
    /// ([`Msg::FeedOpened`]/[`Msg::FeedClosed`]) are shared between Monitor
    /// and Pub/Sub and need the two-state token routing
    /// (`crate::update::feed`).
    PubSubMessage {
        token: crate::command::FeedToken,
        at_ms: u64,
        channel: Vec<u8>,
        via: Option<Vec<u8>>,
        payload: Vec<u8>,
    },
    /// A `Command::UpdateSubscription` add or remove failed on the server
    /// (`docs/plans/m3-pubsub.md` — R7.4: errors surface as a non-blocking
    /// notification carrying the failing command, never a silently dropped
    /// `Err`). `command` is which of `SUBSCRIBE`/`PSUBSCRIBE`/`UNSUBSCRIBE`/
    /// `PUNSUBSCRIBE` failed. `subs` are exactly the
    /// [`crate::state::Subscription`]s that command applied to — for an add
    /// failure these must not stay shown as subscribed (the server never
    /// actually subscribed them), so the core drops any of them still
    /// present in `state.pubsub.subscriptions`; for a remove failure there
    /// is nothing to restore (the chip was already removed locally when the
    /// remove was requested), so this is notification-only.
    SubscriptionFailed {
        command: String,
        detail: String,
        at_ms: u64,
        subs: Vec<crate::state::Subscription>,
    },
}
