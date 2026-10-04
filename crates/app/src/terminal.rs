//! The terminal shell: raw mode, the event loop, and resize handling (PLAN M0.5).
//!
//! The render loop never does I/O. Crossterm events are translated into
//! [`Msg`]s, the core decides what they mean, and the resulting state is drawn.
//! A keystroke is answered within one frame regardless of what the network is
//! doing, because nothing here waits on the network.

use std::io::{Stdout, stdout};
use std::ops::ControlFlow;
use std::sync::Arc;

use crossterm::event::{
    self, Event, KeyCode as XKeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::{execute, terminal};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use fred::interfaces::EventInterface;
use fred::prelude::Client;
use redis_pane_core::clock::Clock;
use redis_pane_core::command::{Command, FeedKindMsg, FeedToken, ReadToken};
use redis_pane_core::key::KeyName;
use redis_pane_core::msg::{KeyCode, KeyPress, MouseAction};
use redis_pane_core::mutation::Mutation;
use redis_pane_core::resolve::Credentials;
use redis_pane_core::theme::{ColorDepth, Theme};
use redis_pane_core::{Msg, State, render, update};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::redis::Established;
use crate::redis::feed::{FeedHandle, FeedKind};
use crate::redis::read::{Arming, ReadGate};

/// Restores the terminal on drop, including when the process is unwinding.
/// Leaving a terminal in raw mode after a panic is the rudest thing a TUI can
/// do to someone at 3am.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            stdout(),
            event::DisableBracketedPaste,
            event::DisableMouseCapture,
            terminal::LeaveAlternateScreen
        );
    }
}

/// Translate a crossterm key into the core's terminal-free representation.
///
/// This function is the boundary: everything above it speaks crossterm,
/// everything below it does not.
pub fn translate(key: KeyEvent) -> Option<Msg> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let code = match key.code {
        XKeyCode::Char(c) => KeyCode::Char(c),
        XKeyCode::Enter => KeyCode::Enter,
        XKeyCode::Esc => KeyCode::Esc,
        XKeyCode::Tab => KeyCode::Tab,
        XKeyCode::Backspace => KeyCode::Backspace,
        XKeyCode::Up => KeyCode::Up,
        XKeyCode::Down => KeyCode::Down,
        XKeyCode::Left => KeyCode::Left,
        XKeyCode::Right => KeyCode::Right,
        XKeyCode::Home => KeyCode::Home,
        XKeyCode::End => KeyCode::End,
        XKeyCode::PageUp => KeyCode::PageUp,
        XKeyCode::PageDown => KeyCode::PageDown,
        XKeyCode::Delete => KeyCode::Delete,
        // Only `F1` has a meaning yet (contextual help, M3) — every other
        // function key still translates, on the same "dropped rather than
        // guessed at" principle the wildcard arm below applies to keys with
        // no `XKeyCode` mapping at all: an unbound `F(n)` reaching the core
        // is a no-op there (`Keymap::action_for` finds nothing), exactly
        // like any other key nothing is bound to.
        XKeyCode::F(n) => KeyCode::F(n),
        _ => return None,
    };
    Some(Msg::Key(KeyPress {
        code,
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
    }))
}

/// Translate a crossterm mouse event the same way [`translate`] does for the
/// keyboard (R7.3). Only the left button is modelled — see
/// [`redis_pane_core::msg::MouseAction`] for why.
pub fn translate_mouse(mouse: MouseEvent) -> Option<Msg> {
    let (col, row) = (mouse.column, mouse.row);
    let action = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => MouseAction::Down { col, row },
        MouseEventKind::Up(MouseButton::Left) => MouseAction::Up,
        MouseEventKind::Drag(MouseButton::Left) => MouseAction::Drag { col, row },
        MouseEventKind::ScrollUp => MouseAction::ScrollUp { col, row },
        MouseEventKind::ScrollDown => MouseAction::ScrollDown { col, row },
        _ => return None,
    };
    Some(Msg::Mouse(action))
}

/// The terminal as the loop draws to it.
type Term = Terminal<CrosstermBackend<Stdout>>;

/// Run the event loop until the core says to quit.
///
/// The render loop never does I/O. Redis work happens on tokio tasks that send
/// [`Msg`]s into this loop; the loop reads state and draws. A keystroke is
/// therefore answerable in one frame regardless of what the network is doing.
#[allow(clippy::too_many_arguments)] // the shell's whole startup hand-off; a struct would only move the list
pub async fn run(
    mut state: State,
    theme: Theme,
    clock: Arc<dyn Clock>,
    client: Client,
    established: Established,
    dial: String,
    credentials: Credentials,
    session_store: Option<crate::state_file::Store>,
) -> std::io::Result<()> {
    terminal::enable_raw_mode()?;
    execute!(
        stdout(),
        terminal::EnterAlternateScreen,
        event::EnableMouseCapture,
        event::EnableBracketedPaste
    )?;
    let _guard = Guard;

    let mut term: Term = Terminal::new(CrosstermBackend::new(stdout()))?;

    let (tx, mut rx) = mpsc::channel::<Msg>(256);

    // Keyboard reads block, so they live on their own thread and arrive as
    // messages like everything else. A plain blocking `event::read()` —
    // nothing here ever hands the terminal to a child process the way the
    // `$EDITOR` escape hatch (Phase 2, `m2-editor-escape-hatch`) will, so
    // there is no second reader to avoid racing for the same fd.
    let input_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            match event::read() {
                Ok(Event::Key(k)) => {
                    if let Some(msg) = translate(k)
                        && input_tx.blocking_send(msg).is_err()
                    {
                        return;
                    }
                }
                Ok(Event::Mouse(m)) => {
                    if let Some(msg) = translate_mouse(m)
                        && input_tx.blocking_send(msg).is_err()
                    {
                        return;
                    }
                }
                Ok(Event::Resize(cols, rows)) => {
                    if input_tx.blocking_send(Msg::Resized { cols, rows }).is_err() {
                        return;
                    }
                }
                Ok(Event::Paste(text)) => {
                    if input_tx.blocking_send(Msg::Paste(text)).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        }
    });

    let size = term.size()?;
    (state, _) = update(
        state,
        Msg::Connected {
            version: established.version.to_string(),
            tracking_supported: established.tracking_supported,
        },
    );
    // A refused INFO at startup must be as visible as one after a reconnect
    // (review M1) — the replica guard failing open is silent otherwise.
    if let Some(detail) = established.server_state_error.clone() {
        (state, _) = update(
            state,
            Msg::Failed {
                command: "INFO (replica and write-rejection checks)".into(),
                detail,
                at_ms: clock.now_epoch_ms(),
            },
        );
    }
    (state, _) = update(
        state,
        Msg::Resized {
            cols: size.width,
            rows: size.height,
        },
    );

    // Session restore (M4 task 7): changes to the restorable part of `State`
    // are written off the render path, debounced; see `state_file`. A failed
    // write is an error notification carrying what failed (R7.4).
    let mut persister = session_store.map(|store| {
        let errors = tx.clone();
        let error_clock = clock.clone();
        crate::state_file::Persister::new(
            store,
            state.session_snapshot(),
            clock.clone(),
            move |detail| {
                let _ = errors.try_send(Msg::Failed {
                    command: "saving session state".into(),
                    detail,
                    at_ms: error_clock.now_epoch_ms(),
                });
            },
        )
    });

    let (landed_tx, mut landed_rx) = mpsc::channel::<(Client, Established)>(1);
    let (feed_landed_tx, mut feed_landed_rx) = mpsc::channel(1);
    let mut shell = Shell {
        client,
        tx,
        clock: clock.clone(),
        read_gate: ReadGate::default(),
        scan_cancel: None,
        reconnect: Reconnect {
            dial,
            credentials,
            attempt: 0,
            cancel: None,
            landed: landed_tx,
        },
        feed: Feed {
            handle: None,
            dialing: None,
            landed: feed_landed_tx,
        },
    };
    shell.start_scan(None);
    shell.watch_link();

    // The Dashboard's poll interval (decision 1, `docs/plans/m3-dashboard.md`):
    // 2 seconds, always ticking. Every tick produces `Msg::DashboardPollTick`
    // and nothing more — the core (`update::dashboard::dashboard_poll_tick`)
    // is the one place that decides whether a given tick actually issues
    // `Command::FetchServerInfo`, reading `View::Dashboard`, the link, and
    // whether a poll is already in flight from state alone. That keeps the
    // decision testable without a real clock or a real interval, and it is
    // what makes this timer's mere existence harmless: a tick while the
    // reader is elsewhere in the app is a `Msg` that changes nothing.
    // Always ticking rather than started/stopped on `View::Dashboard`
    // transitions, for the same reason a feed connection would not want a
    // `tokio::time::Interval` that is only sometimes polled: it accrues
    // missed ticks and bursts through them on return under its default
    // `MissedTickBehavior`. The core never owns this timer (CLAUDE.md's
    // injected-clock discipline) — only the tick, never the decision.
    let mut dashboard_poll = tokio::time::interval(std::time::Duration::from_secs(2));
    dashboard_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Redraw coalescing (M4 task 3, `docs/plans/m4-perf-scan.md` decision
    // 4): the loop used to call `term.draw` unconditionally on every single
    // iteration — once per `Msg`, including every one of the ~2,000
    // `Msg::ScanBatch`es a 1M-key scan produces, and once a second from the
    // countdown tick below even when nothing had changed. `render::frame`
    // is pure, but it still walks the whole of `State` to build a frame, so
    // that was real CPU spent on frames nobody was about to see, on top of
    // the terminal write itself.
    //
    // The fix is a dirty flag plus a minimum frame interval, not a fixed
    // tick: every `update()` sets `dirty`, and the top of the loop draws
    // when `dirty` is set *and* at least `FRAME` has passed since the last
    // draw. A keystroke after any pause is therefore drawn immediately —
    // waiting for a 16ms tick would spend the very budget (PRD §7, "a
    // keystroke must be answerable in one frame") the coalescing exists to
    // protect. Under a flood (scan batches arriving back to back) the same
    // check caps drawing at one frame per `FRAME`, and because it runs at
    // the top of every iteration rather than in a `select!` arm, a flood
    // can never starve it. When a change arrives inside the interval, the
    // `frame_due` arm wakes the loop once the interval is up; it is only
    // armed while `dirty`, so an idle screen does not wake 60 times a
    // second for nothing.
    //
    // The old 1-second countdown arm (which forced a redraw so a TTL
    // countdown visibly moves on an otherwise idle screen) now just sets
    // `dirty`. `update()` is still never called on it — the countdown is a
    // function of the injected clock and the render pass alone (R3.9).
    const FRAME: std::time::Duration = std::time::Duration::from_millis(16);
    let mut countdown_tick = tokio::time::interval(std::time::Duration::from_secs(1));
    countdown_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut dirty = true;
    let mut last_draw: Option<tokio::time::Instant> = None;

    'session: loop {
        let now = tokio::time::Instant::now();
        if dirty && last_draw.is_none_or(|at| now.duration_since(at) >= FRAME) {
            term.draw(|f| {
                let buf = render::frame(&state, &theme, clock.as_ref(), f.area());
                *f.buffer_mut() = buf;
            })?;
            dirty = false;
            last_draw = Some(now);
        }
        let frame_due = last_draw.map_or(now, |at| at + FRAME);
        let persist_due = persister
            .as_ref()
            .and_then(crate::state_file::Persister::deadline)
            .map(tokio::time::Instant::from_std);

        let msg = tokio::select! {
            // Messages first, as before coalescing: input and replies are
            // what the reader is waiting on. Drawing is never starved by
            // this, since it happens at the top of the loop, not here.
            biased;
            msg = rx.recv() => msg,
            () = tokio::time::sleep_until(frame_due), if dirty => continue,
            () = tokio::time::sleep_until(persist_due.unwrap_or(now)), if persist_due.is_some() => {
                if let Some(p) = persister.as_mut() {
                    p.flush_if_due(std::time::Instant::now());
                }
                continue;
            }
            // Ratatui diffs the buffer before writing to the terminal, so a
            // frame where only a few digits changed writes only those
            // cells — this just has to ask for that frame to happen.
            _ = countdown_tick.tick() => {
                dirty = true;
                continue;
            }
            Some((client, established)) = landed_rx.recv() => {
                Some(shell.reconnected(client, &established))
            },
            Some((token, result)) = feed_landed_rx.recv() => {
                match shell.feed_landed(token, result) {
                    Some(msg) => Some(msg),
                    // A dial superseded by a newer `Command::OpenFeed` or
                    // abandoned by `Command::CloseFeed` before it landed —
                    // already torn down inside `feed_landed`, nothing for
                    // `update()` to hear about.
                    None => continue,
                }
            },
            _ = dashboard_poll.tick() => Some(Msg::DashboardPollTick),
        };
        let Some(msg) = msg else {
            break 'session;
        };
        let commands;
        (state, commands) = update(state, msg);
        dirty = true;
        if let Some(p) = persister.as_mut() {
            p.observe(std::time::Instant::now(), state.session_snapshot());
        }

        for command in commands {
            if shell.execute(command, &state, &mut term).await.is_break() {
                break 'session;
            }
        }
    }

    // Quit flushes a pending debounced write rather than losing it. The
    // terminal is restored first so a failure can be printed to stderr.
    let final_session = state.session_snapshot();
    drop(_guard);
    if let Some(p) = persister.take()
        && let Some(detail) = p.finish(final_session)
    {
        eprintln!("redis-pane: could not save session state: {detail}");
    }
    Ok(())
}

/// What the shell holds while the loop runs (review H1).
///
/// The connection, the channel back into the core, the clock, and every
/// in-flight handle, in one place: each [`Command`] is carried out by a method
/// with the context it needs, rather than by a free function taking eight
/// arguments or an arm of a five-hundred-line loop.
struct Shell {
    client: Client,
    tx: mpsc::Sender<Msg>,
    /// Every timestamp a message carries comes from here, never from an
    /// inline `SystemTime::now()`, so there is one clock to inject (review H4).
    clock: Arc<dyn Clock>,
    /// Reads take turns, and superseded ones never reach the wire. See
    /// [`ReadGate`] for why that is a liveness invariant and not a tidiness
    /// preference.
    read_gate: ReadGate,
    scan_cancel: Option<CancellationToken>,
    reconnect: Reconnect,
    /// The single open feed connection, if any (`docs/plans/m3-feed-connection.md`).
    /// Never in `State` — the core holds no `fred` types (ADR-0011). Only one
    /// at a time: the same two-pane screen budget that keeps this a single
    /// slot in the core (`crate::state::FeedStatus`) keeps it a single slot
    /// here too.
    feed: Feed,
}

/// A feed connection's shell-local lifecycle: the dial resolves
/// asynchronously (network I/O, so it must not block the render loop), but
/// the resulting [`FeedHandle`] has to land back on `Shell` — owned by the
/// loop in `run` — before anything can cancel it. `landed` is that handoff,
/// the same shape [`Reconnect::landed`] already uses for the main
/// connection's own async dial.
struct Feed {
    handle: Option<FeedHandle>,
    /// The token of the dial currently in flight, if any — compared against
    /// a landed result so a dial superseded by a newer `Command::OpenFeed` (or
    /// abandoned by a `Command::CloseFeed`) before it finished is torn down on
    /// arrival rather than silently becoming the feed nobody asked for anymore.
    dialing: Option<FeedToken>,
    landed: mpsc::Sender<(FeedToken, Result<FeedHandle, crate::redis::ConnectError>)>,
}

/// Retrying a dropped link (ADR-0009).
struct Reconnect {
    dial: String,
    credentials: Credentials,
    /// How many attempts this outage has cost, purely for the backoff curve
    /// and the header's countdown (`Msg::ReconnectScheduled`) — reset to 0 the
    /// moment a reconnect actually lands. `Command::Reconnect` carries no
    /// attempt number of its own; this is the one place that counts.
    attempt: u32,
    /// Superseded exactly like [`Shell::scan_cancel`]: a manual retry (`r`
    /// while disconnected) must cancel whatever backoff sleep or in-flight
    /// attempt was already running rather than race it.
    cancel: Option<CancellationToken>,
    /// Where a successful attempt's new `Client` goes. A `Client` cannot travel
    /// through a `Msg` without giving the core a dependency on `fred`, which
    /// the core/shell boundary (ADR-0011) forbids, so the loop polls this
    /// channel directly.
    landed: mpsc::Sender<(Client, Established)>,
}

impl Shell {
    /// Carry out one command. `Break` means the core asked to quit.
    async fn execute(
        &mut self,
        command: Command,
        state: &State,
        term: &mut Term,
    ) -> ControlFlow<()> {
        match command {
            Command::Quit => return ControlFlow::Break(()),
            Command::StartScan { pattern } => self.start_scan(pattern),
            Command::CancelScan => {
                if let Some(token) = self.scan_cancel.take() {
                    token.cancel();
                }
            }
            Command::FetchMetadata { indices } => self.fetch_metadata(state, &indices),
            Command::FetchSlowlog { count } => self.fetch_slowlog(count),
            Command::FetchServerInfo { token } => self.fetch_server_info(token),
            Command::ReadKey {
                key,
                index,
                token,
                arm,
            } => self.read_key(key, index, token, arm),
            Command::Execute { mutation, index } => self.mutate(mutation, index),
            Command::CopyToClipboard { text, label } => self.copy(text, label, term).await,
            Command::Notify { text } => {
                let at_ms = self.clock.now_epoch_ms();
                let _ = self.tx.send(Msg::Noticed { text, at_ms }).await;
            }
            Command::Reconnect { after_ms } => {
                self.reconnect.schedule(after_ms, &self.tx, &self.clock)
            }
            Command::OpenFeed { kind, token } => self.open_feed(kind, token),
            Command::CloseFeed => self.close_feed(),
            Command::UpdateSubscription { add, remove } => {
                if let Some(handle) = self.feed.handle.as_ref() {
                    handle.update_subscription(add, remove, self.tx.clone(), self.clock.clone());
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn start_scan(&mut self, pattern: Option<String>) {
        if let Some(previous) = self.scan_cancel.take() {
            previous.cancel();
        }
        let token = CancellationToken::new();
        self.scan_cancel = Some(token.clone());
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            crate::redis::scan::stream_keys(&client, pattern.as_deref(), tx, token).await;
        });
    }

    fn fetch_metadata(&self, state: &State, indices: &[usize]) {
        // Resolve names here, on the UI side, so the task owns no reference
        // into state.
        let window: Vec<(usize, Vec<u8>)> = indices
            .iter()
            .filter_map(|i| state.keys.name(*i).map(|n| (*i, n.to_vec())))
            .collect();
        if window.is_empty() {
            return;
        }
        let (client, tx, clock) = (self.client.clone(), self.tx.clone(), self.clock.clone());
        tokio::spawn(async move {
            let at_ms = clock.now_epoch_ms();
            let msg = match crate::redis::fetch_metadata(&client, &window).await {
                Ok((entries, gone)) if entries.is_empty() && gone.is_empty() => return,
                Ok((entries, gone)) => Msg::MetadataBatch {
                    entries,
                    gone,
                    at_ms,
                },
                Err(e) => Msg::Failed {
                    command: "fetching metadata".into(),
                    detail: e.details().to_string(),
                    at_ms: clock.now_epoch_ms(),
                },
            };
            let _ = tx.send(msg).await;
        });
    }

    /// Fetch the slowlog ring buffer (R6.4, M3). No arming, no liveness — a
    /// plain request/response, unlike [`Shell::read_key`].
    fn fetch_slowlog(&self, count: i64) {
        let (client, tx, clock) = (self.client.clone(), self.tx.clone(), self.clock.clone());
        tokio::spawn(async move {
            let msg = match crate::redis::read::fetch_slowlog(&client, count).await {
                Ok(entries) => Msg::SlowlogLoaded { entries },
                Err(e) => Msg::SlowlogFailed {
                    detail: e.details().to_string(),
                    at_ms: clock.now_epoch_ms(),
                },
            };
            let _ = tx.send(msg).await;
        });
    }

    /// Fetch the server's own vitals (R6.3, M3 task 6,
    /// `docs/plans/m3-dashboard.md`). No arming, no liveness, matching
    /// [`Shell::fetch_slowlog`] — a plain request/response. `token` is
    /// [`Command::FetchServerInfo`]'s own — minted by the core, carried
    /// through unchanged, and echoed back on both replies so
    /// `update::dashboard::server_info_loaded`/`server_info_failed` can drop
    /// a stale one (review item (b)): the shell holds no opinion about
    /// which fetch is "current," it only ever answers the one it was asked
    /// to make.
    fn fetch_server_info(&self, token: redis_pane_core::command::InfoToken) {
        let (client, tx, clock) = (self.client.clone(), self.tx.clone(), self.clock.clone());
        tokio::spawn(async move {
            let msg = match crate::redis::read::fetch_server_info(&client).await {
                Ok(info) => Msg::ServerInfoLoaded {
                    info,
                    at_ms: clock.now_epoch_ms(),
                    token,
                },
                Err(e) => Msg::ServerInfoFailed {
                    detail: e.details().to_string(),
                    at_ms: clock.now_epoch_ms(),
                    token,
                },
            };
            let _ = tx.send(msg).await;
        });
    }

    /// Read a key and send the result in. The one read path: it always re-arms
    /// where arming is possible, so there is no branch on which liveness can be
    /// silently lost (ADR-0006).
    ///
    /// Every `Command::ReadKey` lands here: an Open, a manual Refetch, and the
    /// one an invalidation push triggers. That is what unifies both re-arm
    /// invariants (ADR-0006, ADR-0009) into one place instead of two. Whether to
    /// arm is the command's `arm`, decided by the core (review H3); the key is the
    /// command's exact bytes, never rebuilt from display text (review C1).
    ///
    /// **Found by testing against real managed servers, not by any test in the
    /// suite:** `CLIENT CACHING YES` was sent and awaited inside `read_value`, but
    /// nothing ever told the core it had succeeded. `State::liveness()` correctly
    /// refuses to report `Live` without a `Msg::TrackingArmed` — that guard is the
    /// whole point of ADR-0009 — but nothing on this path ever sent one. The header
    /// read `○ manual` forever, on every server, including local Redis with
    /// tracking fully working. The core's invariant was airtight; the shell simply
    /// never told it the truth.
    fn read_key(&mut self, key: KeyName, index: Option<usize>, token: ReadToken, arm: bool) {
        // Supersede whatever was in flight. The core would ignore its reply anyway
        // — every reply carries the token of the read it answers — but ignoring a
        // reply does not un-send the `CLIENT CACHING YES` that came with it, and
        // that is the half which decides what the server tracks.
        let permit = self.read_gate.begin();
        let arming = if arm {
            Arming::Enabled
        } else {
            Arming::Unsupported
        };
        let (client, tx, clock) = (self.client.clone(), self.tx.clone(), self.clock.clone());

        // Stamps the loading indicator's delay gate (`PendingRead::APPEAR_DELAY_MS`,
        // `crates/core/src/state/open.rs`). `update()` has no clock of its own
        // (ADR-0011); this is the one moment the shell actually knows when the
        // read was dispatched. Best-effort: a dropped send (the channel full)
        // just leaves the read unstamped, which the render layer already treats
        // as "not yet worth mentioning" — never a crash, never a wrong timestamp.
        let _ = tx.try_send(Msg::ReadIssued {
            token,
            at_ms: clock.now_epoch_ms(),
        });

        tokio::spawn(async move {
            let at_ms = clock.now_epoch_ms();
            let Some(result) = permit
                .run(crate::redis::read::read_value(
                    &client,
                    key.as_bytes(),
                    arming,
                ))
                .await
            else {
                return;
            };

            // `read_value` sends `CLIENT CACHING YES` in one pipeline with its
            // first read and returns early with `?` if either fails, so any Ok(_)
            // here means arming already succeeded on the wire — this message is
            // what makes that fact reach the core.
            if arming == Arming::Enabled && result.is_ok() {
                let _ = tx.send(Msg::TrackingArmed).await;
            }

            let msg = match result {
                Ok(Some(read)) => Msg::ValueLoaded {
                    token,
                    index,
                    name: key,
                    value: read.value,
                    ttl_seconds: read.ttl_seconds,
                    size_bytes: read.size_bytes,
                    at_ms,
                },
                Ok(None) => Msg::ValueGone {
                    token,
                    index,
                    name: key,
                    at_ms,
                },
                // Never swallowed: a Redis error that produces no visible effect is
                // indistinguishable from the app deciding to do nothing (R7.4).
                //
                // Deliberately carries no token, so it is never dropped as stale. A
                // read that failed, failed — the key it names is in the message, and
                // suppressing it because the reader has moved on would be swallowing
                // an error on a technicality (R7.4).
                Err(e) => Msg::Failed {
                    command: format!("reading {key}"),
                    detail: e.details().to_string(),
                    at_ms,
                },
            };
            let _ = tx.send(msg).await;
        });
    }

    /// Execute a confirmed write and report how it settled (review H1).
    ///
    /// What the outcome means is the core's to decide; this only carries it
    /// back, with the mutation it answers.
    fn mutate(&self, mutation: Mutation, index: Option<usize>) {
        let (client, tx, clock) = (self.client.clone(), self.tx.clone(), self.clock.clone());
        tokio::spawn(async move {
            let result = crate::redis::mutate::execute(&client, &mutation)
                .await
                .map_err(|e| e.details().to_string());
            let at_ms = clock.now_epoch_ms();
            let _ = tx
                .send(Msg::MutationSettled {
                    mutation,
                    index,
                    result,
                    at_ms,
                })
                .await;
        });
    }

    async fn copy(&self, text: String, label: String, term: &mut Term) {
        let method = crate::clipboard::detect_method();
        // Truncation is an OSC-52-only concern (native has no comparable cap),
        // and only OSC 52 writes to our own stdout, so only it needs a redraw
        // after.
        let truncated =
            method == crate::clipboard::Method::Osc52 && crate::clipboard::was_truncated(&text);
        let result = crate::clipboard::copy(&text, method);
        if method == crate::clipboard::Method::Osc52 {
            let _ = term.clear();
        }
        let at_ms = self.clock.now_epoch_ms();
        let msg = match result {
            Ok(()) => Msg::Copied {
                label: if truncated {
                    format!("{label}, truncated")
                } else {
                    label
                },
                at_ms,
            },
            // A copy that failed used to produce nothing at all — no notice, no
            // error — which is indistinguishable from one that worked, and leaves
            // the reader pasting whatever was on the clipboard before (R7.4). OSC
            // 52 can still be dropped by the terminal without telling anyone;
            // that is a limit of the protocol. This is the half we can see.
            Err(e) => Msg::Failed {
                command: "copying to the clipboard".into(),
                detail: e.to_string(),
                at_ms,
            },
        };
        let _ = self.tx.send(msg).await;
    }

    /// `Command::OpenFeed`: dial a second connection to the same resolved
    /// target and start streaming from it (`docs/plans/m3-feed-connection.md`).
    ///
    /// Every `g m`/`g p` reopens a fresh connection rather than resuming a
    /// stale one (the plan's own rule) — so an already-open feed is closed
    /// first, exactly like [`Shell::start_scan`] supersedes an in-flight scan
    /// before starting a new one. The dial itself is network I/O and must not
    /// block the render loop (CLAUDE.md), so it runs on its own task and the
    /// result lands back on `self.feed.landed`, the same handoff
    /// [`Reconnect::landed`] uses for the main connection's own async dial.
    fn open_feed(&mut self, kind: FeedKindMsg, token: FeedToken) {
        if let Some(handle) = self.feed.handle.take() {
            handle.close();
        }
        self.feed.dialing = Some(token);

        let kind = match kind {
            FeedKindMsg::Monitor => FeedKind::Monitor,
            FeedKindMsg::Subscribe(subs) => FeedKind::Subscribe(subs),
        };
        let (dial, credentials) = (
            self.reconnect.dial.clone(),
            self.reconnect.credentials.clone(),
        );
        let (tx, clock, landed) = (
            self.tx.clone(),
            self.clock.clone(),
            self.feed.landed.clone(),
        );
        let main = self.client.clone();
        tokio::spawn(async move {
            let result =
                crate::redis::feed::open_feed(&dial, &credentials, kind, token, tx, clock, &main)
                    .await;
            let _ = landed.send((token, result)).await;
        });
    }

    /// `Command::CloseFeed`: tear down whatever feed connection is open.
    /// `Command::CancelScan`'s sibling for the second connection — cancel and
    /// drop, nothing cleverer. A no-op if none is open, including one still
    /// mid-dial: clearing `dialing` here is what makes [`Shell::feed_landed`]
    /// close a dial that finishes after the reader already left the view,
    /// rather than silently reopening it.
    fn close_feed(&mut self) {
        self.feed.dialing = None;
        if let Some(handle) = self.feed.handle.take() {
            handle.close();
        }
    }

    /// A feed dial finished, successfully or not. Applied here, in the loop's
    /// own task rather than the spawned one, for the same reason
    /// [`Shell::reconnected`] is: `self.feed.handle` must be current before
    /// any `Msg` this returns reaches `update()`.
    ///
    /// Returns `None` for a dial that is no longer the one anybody is waiting
    /// on — superseded by a newer `Command::OpenFeed`, or abandoned by a
    /// `Command::CloseFeed` before it landed — after tearing down whatever it
    /// produced rather than leaking it or resurrecting a feed the reader
    /// already left.
    fn feed_landed(
        &mut self,
        token: FeedToken,
        result: Result<crate::redis::feed::FeedHandle, crate::redis::ConnectError>,
    ) -> Option<Msg> {
        if self.feed.dialing != Some(token) {
            if let Ok(handle) = result {
                handle.close();
            }
            return None;
        }
        self.feed.dialing = None;
        match result {
            Ok(handle) => {
                self.feed.handle = Some(handle);
                Some(Msg::FeedOpened { token })
            }
            Err(err) => Some(Msg::FeedClosed {
                token,
                reason: Some(err.to_string()),
            }),
        }
    }

    /// A reconnect that landed.
    ///
    /// Swapped in here, in the same task as everything else in the loop, rather
    /// than inside the spawned task that found it — so `client` is current
    /// *before* the `Msg::Connected` this returns is ever handed to `update()`,
    /// and so the `Command::ReadKey` that `Msg::Connected` asks for reads the new
    /// connection rather than the dead one. Ordering here is not incidental: the
    /// same guarantee through the message channel would need `Msg::Connected` to
    /// arrive only after the client was already swapped, and two independent
    /// channels give no such promise about which is drained first.
    fn reconnected(&mut self, client: Client, established: &Established) -> Msg {
        self.client = client;
        self.reconnect.attempt = 0;
        // Subscriptions are tied to the `Client` they were opened on; one that
        // has been replaced no longer delivers anything.
        self.watch_link();
        Msg::Connected {
            version: established.version.to_string(),
            tracking_supported: established.tracking_supported,
        }
    }

    /// Subscribe to the client's link-level streams: reconnect notifications,
    /// wire errors, and invalidation pushes. Tied to the `Client` instance, so
    /// this runs again after every successful reconnect.
    fn watch_link(&self) {
        // fred reconnects underneath us on its own schedule if it has a
        // `ReconnectPolicy` (this app sets none, so in practice this fires only
        // if that ever changes) — and the server on the other side remembers
        // nothing about what we were watching. Telling the core lets it drop the
        // liveness claim and re-arm, the invariant ADR-0009 exists for.
        {
            let mut reconnects = self.client.reconnect_rx();
            let tx = self.tx.clone();
            let probe = self.client.clone();
            let clock = self.clock.clone();
            tokio::spawn(async move {
                while reconnects.recv().await.is_ok() {
                    // A fresh connection tracks nothing, so capability must be
                    // re-probed rather than remembered.
                    let tracking = crate::redis::probe_tracking(&probe).await;
                    // A refused INFO here must be as visible as any other
                    // failure — defaulting to "not a replica" is the silent
                    // failure ADR-0009 exists to prevent (review M1).
                    match crate::redis::server_conditions(&probe).await {
                        Ok((read_only, condition)) => {
                            let _ = tx
                                .send(Msg::ServerState {
                                    read_only,
                                    condition,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(Msg::Failed {
                                    command: "INFO (replica and write-rejection checks)".into(),
                                    detail: e.details().to_string(),
                                    at_ms: clock.now_epoch_ms(),
                                })
                                .await;
                        }
                    }
                    if tx
                        .send(Msg::Connected {
                            version: String::new(),
                            tracking_supported: tracking,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
        {
            let mut errors = self.client.error_rx();
            let tx = self.tx.clone();
            let clock = self.clock.clone();
            tokio::spawn(async move {
                while let Ok((error, _server)) = errors.recv().await {
                    // A connection-level error means the link is gone; anything
                    // else is a command failure and belongs in a notification.
                    //
                    // Both kinds matched here are what `connect_with`'s bounded-
                    // responsiveness config (`crates/app/src/redis/mod.rs`) can
                    // actually produce for a silently dead socket: `IO` from
                    // fred's own unresponsive-connection detector
                    // (`fred-10.1.0/src/router/types.rs:58`), and `Timeout` from
                    // the per-command `default_command_timeout` backstop
                    // (`fred-10.1.0/src/utils.rs:286-299`). The latter is
                    // ordinarily returned straight to the awaiting caller rather
                    // than broadcast here, but treating it the same on the rare
                    // path where it does arrive on this stream costs nothing —
                    // a command that just timed out on a wire this deliberately
                    // slow to declare unresponsive is not a connection worth
                    // still calling live.
                    let msg = if matches!(
                        error.kind(),
                        fred::error::ErrorKind::IO | fred::error::ErrorKind::Timeout
                    ) {
                        Msg::ConnectionLost
                    } else {
                        Msg::Failed {
                            command: "connection".into(),
                            detail: error.details().to_string(),
                            at_ms: clock.now_epoch_ms(),
                        }
                    };
                    if tx.send(msg).await.is_err() {
                        return;
                    }
                }
            });
        }

        // Invalidation pushes arrive on their own task and become messages like
        // everything else. This is what makes a value update with no keypress —
        // and it is the whole reason this project exists (ADR-0006). Subscribed
        // whether or not the server tracks: a connection that never arms never
        // receives a push, and the capability is the core's to know (review H3).
        {
            let mut invalidations =
                fred::interfaces::TrackingInterface::invalidation_rx(&self.client);
            let tx = self.tx.clone();
            tokio::spawn(async move {
                while let Ok(_invalidation) = invalidations.recv().await {
                    // The server has told us the open key changed, which also
                    // consumed the arming. Refetching is what re-arms, and the core
                    // decides whether the result lands or is announced.
                    if tx.send(Msg::Invalidated).await.is_err() {
                        return;
                    }
                }
            });
        }
    }
}

impl Reconnect {
    /// Attempt one reconnect after `after_ms`, cancelling whatever backoff sleep
    /// or in-flight attempt was already running — `r` while disconnected retries
    /// immediately (ADR-0009) rather than racing the timer.
    ///
    /// Success reaches the loop through [`Reconnect::landed`]. Failure is
    /// ordinary shell-reported state — `Msg::Failed` for what went wrong,
    /// `Msg::ReconnectScheduled` for when to try again — and needs no special
    /// channel.
    fn schedule(&mut self, after_ms: u64, tx: &mpsc::Sender<Msg>, clock: &Arc<dyn Clock>) {
        self.attempt += 1;
        if let Some(previous) = self.cancel.take() {
            previous.cancel();
        }
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        let (dial, credentials, attempt) =
            (self.dial.clone(), self.credentials.clone(), self.attempt);
        let (landed, tx, clock) = (self.landed.clone(), tx.clone(), clock.clone());
        tokio::spawn(async move {
            tokio::select! {
                biased;
                () = token.cancelled() => {}
                () = async {
                    tokio::time::sleep(std::time::Duration::from_millis(after_ms)).await;
                    match crate::redis::connect_with(&dial, &credentials).await {
                        Ok((client, established)) => {
                            // A refused INFO on this connection must be as
                            // visible as it is at startup (review M1); it must
                            // not be quietly presented as a clean ServerState.
                            match &established.server_state_error {
                                None => {
                                    let _ = tx
                                        .send(Msg::ServerState {
                                            read_only: established.read_only,
                                            condition: established.condition,
                                        })
                                        .await;
                                }
                                Some(detail) => {
                                    let _ = tx
                                        .send(Msg::Failed {
                                            command: "INFO (replica and write-rejection checks)"
                                                .into(),
                                            detail: detail.clone(),
                                            at_ms: clock.now_epoch_ms(),
                                        })
                                        .await;
                                }
                            }
                            let _ = landed.send((client, established)).await;
                        }
                        Err(err) => {
                            let next = attempt + 1;
                            let retry_in_ms = crate::redis::backoff_for(next).as_millis() as u64;
                            let _ = tx
                                .send(Msg::Failed {
                                    command: "reconnecting".into(),
                                    detail: err.to_string(),
                                    at_ms: clock.now_epoch_ms(),
                                })
                                .await;
                            let _ = tx
                                .send(Msg::ReconnectScheduled {
                                    attempt: next,
                                    retry_in_ms,
                                })
                                .await;
                        }
                    }
                } => {}
            }
        });
    }
}

/// What the terminal can display. A real probe belongs in M0.3's follow-up;
/// `COLORTERM` is the part that is both cheap and reliable.
pub fn detect_color_depth() -> ColorDepth {
    resolve_color_depth(ColorEnv {
        no_color: std::env::var("NO_COLOR").ok(),
        colorterm: std::env::var("COLORTERM").ok(),
        term: std::env::var("TERM").ok(),
        wt_session: std::env::var("WT_SESSION").ok(),
        windows: cfg!(windows),
    })
}

/// The environment [`resolve_color_depth`] decides from.
struct ColorEnv {
    no_color: Option<String>,
    colorterm: Option<String>,
    term: Option<String>,
    /// Set by Windows Terminal, which renders truecolor but sets neither
    /// `COLORTERM` nor `TERM`.
    wt_session: Option<String>,
    windows: bool,
}

/// The pure decision, taken out of [`detect_color_depth`] so it can be tested
/// without mutating the process environment — this workspace forbids
/// `unsafe`, which `std::env::set_var` requires in edition 2024, so a function
/// that reads the environment directly cannot be unit tested at all.
///
/// Severity-4 UI task: this had no dedicated tests before, only the golden
/// frames' assertion that each `ColorDepth` renders distinctly once selected —
/// nothing pinned *which* depth a given environment resolves to. `NO_COLOR`
/// (<https://no-color.org>) is now honoured too: a user who has set it wants
/// monochrome regardless of what the terminal claims to support, and ignoring
/// it was the one real gap here.
///
/// `COLORTERM` and `TERM` are Unix conventions: PowerShell, conhost and
/// Windows Terminal set neither, so reading their absence as "declares
/// nothing" rendered every Windows session monochrome. Every console since
/// Windows 10 1703 interprets VT colour sequences (crossterm enables them), so
/// on Windows an absent `TERM` means 256 colours, and `WT_SESSION` means
/// truecolor.
fn resolve_color_depth(env: ColorEnv) -> ColorDepth {
    if env.no_color.is_some() {
        return ColorDepth::Monochrome;
    }
    if matches!(env.colorterm.as_deref(), Some("truecolor") | Some("24bit"))
        || env.wt_session.is_some()
    {
        return ColorDepth::TrueColor;
    }
    match env.term.as_deref() {
        Some(t) if t.contains("256") => ColorDepth::Ansi256,
        Some("dumb") => ColorDepth::Monochrome,
        None if !env.windows => ColorDepth::Monochrome,
        _ => ColorDepth::Ansi256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(no_color: Option<&str>, colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
        resolve_color_depth(ColorEnv {
            no_color: no_color.map(String::from),
            colorterm: colorterm.map(String::from),
            term: term.map(String::from),
            wt_session: None,
            windows: false,
        })
    }

    fn windows_depth(no_color: Option<&str>, wt_session: Option<&str>) -> ColorDepth {
        resolve_color_depth(ColorEnv {
            no_color: no_color.map(String::from),
            colorterm: None,
            term: None,
            wt_session: wt_session.map(String::from),
            windows: true,
        })
    }

    #[test]
    fn windows_terminal_is_truecolor_without_colorterm_or_term() {
        assert_eq!(
            windows_depth(None, Some("0b2c7a3e-1f4d-4b8e-9c61-3a5d2e7f8b90")),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn a_windows_console_with_no_term_is_ansi256_not_monochrome() {
        // PowerShell in conhost sets neither COLORTERM nor TERM; it was the
        // session that rendered monochrome.
        assert_eq!(windows_depth(None, None), ColorDepth::Ansi256);
    }

    #[test]
    fn no_color_still_wins_on_windows() {
        assert_eq!(
            windows_depth(Some("1"), Some("some-session")),
            ColorDepth::Monochrome
        );
        assert_eq!(windows_depth(Some("1"), None), ColorDepth::Monochrome);
    }

    #[test]
    fn truecolor_from_colorterm() {
        assert_eq!(depth(None, Some("truecolor"), None), ColorDepth::TrueColor);
        assert_eq!(
            depth(None, Some("24bit"), Some("xterm")),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn ansi256_when_term_says_so() {
        assert_eq!(
            depth(None, None, Some("xterm-256color")),
            ColorDepth::Ansi256
        );
        assert_eq!(
            depth(None, None, Some("screen-256color")),
            ColorDepth::Ansi256
        );
    }

    #[test]
    fn monochrome_for_a_dumb_terminal_or_no_term_at_all() {
        assert_eq!(depth(None, None, Some("dumb")), ColorDepth::Monochrome);
        assert_eq!(depth(None, None, None), ColorDepth::Monochrome);
    }

    #[test]
    fn an_unrecognised_but_present_term_defaults_to_ansi256() {
        // A terminal that sets TERM to something we do not recognise is more
        // likely to support some colour than none; monochrome is reserved for
        // the terminal actively declaring "dumb" or declaring nothing.
        assert_eq!(depth(None, None, Some("screen")), ColorDepth::Ansi256);
        assert_eq!(depth(None, None, Some("vt100")), ColorDepth::Ansi256);
    }

    #[test]
    fn no_color_forces_monochrome_regardless_of_everything_else() {
        // https://no-color.org — a user who sets this wants monochrome even if
        // the terminal is fully capable of truecolor.
        assert_eq!(
            depth(Some("1"), Some("truecolor"), Some("xterm-256color")),
            ColorDepth::Monochrome
        );
        assert_eq!(
            depth(Some(""), None, None),
            ColorDepth::Monochrome,
            "presence is what matters, not the value"
        );
    }

    #[test]
    fn colorterm_wins_over_term_when_both_are_present() {
        assert_eq!(
            depth(None, Some("truecolor"), Some("dumb")),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn a_plain_char_translates() {
        let ev = KeyEvent::new(XKeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(
            translate(ev),
            Some(Msg::Key(KeyPress::plain(KeyCode::Char('q'))))
        );
    }

    #[test]
    fn ctrl_is_carried_across_the_boundary() {
        let ev = KeyEvent::new(XKeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            translate(ev),
            Some(Msg::Key(KeyPress::ctrl(KeyCode::Char('c'))))
        );
    }

    #[test]
    fn key_releases_are_ignored_so_a_press_is_not_counted_twice() {
        let mut ev = KeyEvent::new(XKeyCode::Char('q'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        assert_eq!(translate(ev), None);
    }

    #[test]
    fn an_untranslatable_key_is_dropped_rather_than_guessed_at() {
        // `F(7)` used to be this test's example, before `F1` gained a
        // meaning (contextual help, M3) — every function key translates now,
        // so a media key nothing in the core has a use for takes its place.
        let ev = KeyEvent::new(XKeyCode::CapsLock, KeyModifiers::NONE);
        assert_eq!(translate(ev), None);
    }

    #[test]
    fn f1_translates_for_the_help_alias() {
        let ev = KeyEvent::new(XKeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(
            translate(ev),
            Some(Msg::Key(KeyPress::plain(KeyCode::F(1))))
        );
    }

    /// M0.5's proof: synthetic events drive the core with no terminal attached.
    #[test]
    fn synthetic_events_drive_the_core_to_quit_without_a_terminal() {
        let mut state = State::default();
        let mut commands = Vec::new();
        for ev in [
            KeyEvent::new(XKeyCode::Char('x'), KeyModifiers::NONE),
            KeyEvent::new(XKeyCode::Char('q'), KeyModifiers::NONE),
        ] {
            if let Some(msg) = translate(ev) {
                (state, commands) = update(state, msg);
            }
        }
        assert!(state.quitting);
        assert_eq!(commands, vec![Command::Quit]);
    }
}
