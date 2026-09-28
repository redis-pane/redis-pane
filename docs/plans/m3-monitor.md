# M3 task 4: Monitor (`g m`)

Status: **done, 2026-09-28.** Built in one PR with task 2's feed plumbing
([`m3-feed-connection.md`](m3-feed-connection.md)) as phase A and this doc as phase B. No ADR.
DESIGN §6.7 has been expanded with the built shape (banner, columns, pause/following, filter,
closed-feed/reopen, cost confirmation) rather than the two placeholder sentences this doc
originally quoted. CONTEXT.md's proposed **Monitor** entry (below) has been added.

## Decisions (2026-09-27) — these override the body below where they differ

1. **View state is `state.screen`** (`View::Monitor`), not `state.view` — see the Slowlog task.
   `g m` is a chord in `Keymap.chords`, beside `g k`/`g s`.
2. **Leaving the Monitor view by any route closes the feed** — `Esc`, `g k`, `g s`, quitting. A
   `MONITOR` left running behind another view costs the server with nothing on screen to say so.
   Every `g m` opens a fresh connection; nothing is resumed.
3. **Confirmation on `prod` and `unknown`.** In those Environments `g m` first shows a confirm
   dialog naming `MONITOR` and its cost; `y` opens the view, `Esc` does nothing. `local` and
   `staging` open straight away. This is **not a mutation**: Read-only Mode does not refuse it,
   and it must not be modelled as a `Mutation`/`PendingMutation` that confirm's read-only check
   would reject. Reuse the confirm overlay's rendering and `y`/`Esc` handling with a distinct
   pending kind; choose the least invasive shape the code allows and say why in the PR.
4. **Line cap and byte cap.** `MONITOR_CAP` = 5000 lines, and each line's raw text is truncated
   to `MONITOR_LINE_MAX` = 4 KiB on arrival (marked as truncated) — a single `SET` with a 10MB
   value would otherwise make one line bigger than the whole buffer is meant to be. Worst case is
   ~20MB. Both caps are applied in `push_monitor_line`, the one function that appends.
5. **Pause is `p`**, scoped to the Monitor view (keymap growth rule: view-scoped, not a new bare
   key). Paused lines are counted, not buffered (`paused · 340 skipped`); resume does not
   backfill. The socket stays open while paused.
6. **Filter is `/`**, reusing the keys pane's filter capture and matching semantics — display
   only, never changes what is buffered.
7. **Following the tail:** the view follows new lines while the selection is on the last line.
   Moving up stops following (the header says so, e.g. `following off · End to resume`) without
   pausing consumption; `End`/`G`-equivalent resumes. If the selected line is evicted by the cap,
   the selection clamps to the oldest line.
8. **Columns:** the server's own timestamp from the `MONITOR` line, shown as UTC
   `HH:MM:SS.mmm`, then db, client and the command. Lines are shown with the Viewer's byte
   escaping, never raw. `c` copies the selected line's command.
9. **Feed closed** (server closed it, network drop, `-NOPERM` under ACLs): the view keeps the
   buffer on screen, the header says `feed closed: <reason>`, an R7.4 notification carries the
   command, and `r` reopens (label `reopen`). No automatic reconnect.
10. **Banner:** a warning-token line at the top of the view whenever it is open, at every width
    down to single-pane — not dismissible.

Help gets a `Monitor` context (with the `p`/`/`/`r` rows and `c`), and EVERYWHERE lists `g m`.
ADR-0005 gets a Consequences line: a feed connection is a second socket to the same target, not a
second Connection.

Depends on `docs/plans/m3-feed-connection.md` (the dedicated second connection `MONITOR` needs)
and the `View` enum introduced in `docs/plans/m3-slowlog.md` (the full-screen-view state machine).
Neither is repeated here.

## Context

PLAN.md M3 row 4: "Monitor (`g m`): live tail, filter box, pause/resume, bounded buffer with a
visible cap, persistent cost-warning banner · Proves: buffer never grows unbounded; pausing stops
consuming the feed, not just hides it; the warning is impossible to miss." PRD R6.1: "`MONITOR`
tail with filtering — clearly flagged as expensive."

`MONITOR` streams a line of text per command the server executes, server-wide, for as long as the
connection stays in monitor mode — not scoped to a key, a database, or a client. On a busy server
this is a firehose, and it has a real cost: the server pays to format and ship every command to
every monitoring connection. This is the "clearly flagged as expensive" half of R6.1, and it is
also why `MONITOR` needs its own connection (`m3-feed-connection.md`) rather than sharing the main
one — a `MONITOR`'d connection accepts no other commands until it is closed.

## Architecture

### Opening and the feed

- `g m` sets `state.view = View::Monitor` and emits `Command::OpenFeed { kind:
  FeedKindMsg::Monitor }` (from `m3-feed-connection.md`). The shell's `open_feed` dials a second
  connection and runs `MONITOR`; each streamed line becomes `Msg::MonitorLine { at_ms, raw: String
  }` sent through the same channel every other `Msg` arrives on. `at_ms` comes from the injected
  `Clock` at the point the shell receives the line — ADR-0011's discipline (the clock is injected,
  never read directly by the core) means the shell timestamps on receipt and hands the core a
  plain number, exactly like `Msg::ReadIssued`/`Msg::ReadCompleted` already do for ordinary reads.
- `MONITOR`'s own reply format includes a server-side timestamp per line already
  (`1339518083.107412 [0 127.0.0.1:60866] "keys" "*"`) — worth parsing and preferring over the
  shell's receipt time for display once this is built, but the receipt time is what makes the
  *local* rendering (buffer ordering, "how long has this view been running") a pure function of
  injected state rather than of wall-clock reads scattered through `update()`.

### Bounded buffer — the load-bearing requirement

PLAN's "Proves" column for this row is explicit that the buffer must never grow unbounded and that
pausing must stop *consuming*, not just *hiding*. This is the same discipline ADR-0010 established
for the Loaded set ("every scanned key is retained up to a documented cap... enforced in exactly
one place") applied to a stream instead of a scan:

- `crates/core/src/state/monitor.rs` (new): `MonitorState { lines: VecDeque<MonitorLine>, cap:
  usize, paused: bool, filter: String }`. A `VecDeque` because this is a tail — oldest lines drop
  off the front as new ones arrive past `cap`, the opposite access pattern from the Loaded set's
  arena (which retains everything up to a hard stop). `cap` is a fixed constant for M3 (DESIGN
  gives no number; propose something on the order of a few thousand lines — enough scrollback to
  be useful, small enough that a `MonitorLine { at_ms: u64, raw: String }` buffer stays a rounding
  error next to the Loaded set's 40MB/million-key budget).
- **The cap is enforced in exactly one place**, per ADR-0010's naming discipline extended here: one
  function, `push_monitor_line` in `crates/core/src/update/monitor.rs`, is the only path that adds
  to `MonitorState::lines`, and it pops from the front whenever length would exceed `cap`. No other
  code path touches `lines` directly — mirrors `scan_batch` being the single enforcement point for
  the keys cap (CLAUDE.md: "The cap is enforced in exactly one place").
- **Pause must stop consuming the feed, not just hide the pane.** This is the subtle half of the
  requirement: a naive implementation renders `lines` conditionally on `!paused` while the feed
  task keeps pushing `Msg::MonitorLine`s into `update()` regardless, which still grows `lines`
  unboundedly behind a dropped display — the visible symptom (a frozen view) hides the real bug
  (memory still climbing). The correct shape: `Msg::MonitorLine` while `paused` is **not** appended
  to `lines` at all — it is either dropped, or (better, so resuming does not show a gap the reader
  did not ask for) counted in a `dropped_while_paused: u64` the header can report ("paused · 340
  lines skipped"), but never buffered for later replay — a paused Monitor is not a queue, it is a
  closed valve. This is also why `Command::CloseFeed` is not the pause mechanism: pausing keeps the
  socket open (resuming should not pay reconnection cost or lose `MONITOR`'s own warm-up), it just
  changes what `update()` does with what arrives.

### Filter

- A filter box (DESIGN's "filter box, pause/resume") narrows the *displayed* lines by substring or
  glob over the raw `MONITOR` text — same `/`-opens-filter-capture UX the keys pane already has
  (`state.filtering`, `crates/core/src/update/keys.rs`), reused for consistency rather than
  invented fresh for Monitor. Filtering is a display-time concern over the buffer already held; it
  does not change what gets appended to `lines` (that is `cap`'s and pause's job, not the filter's)
  — a filtered-out line still occupies its slot in the bounded buffer so that clearing the filter
  shows exactly what would have been there anyway.

### The warning banner

DESIGN: "a persistent warning banner on `MONITOR` explaining its cost." Persistent means exactly
that — not a toast that fades (CLAUDE.md's `state.notice` fades on its own; a warning about an
expensive, still-running operation must not disappear while the operation is still expensive and
still running). Render as a fixed header line in the Monitor view's frame, present for the entire
time `state.view == View::Monitor` and the feed is open, not dismissible by anything short of
leaving the view — the same "impossible to miss" requirement PLAN states directly.

## CLAUDE.md rules this binds

- **The cap is enforced in exactly one place.** `push_monitor_line`, see above — this task's
  direct analogue of `scan_batch`.
- **Every in-flight operation must be cancellable (`Esc`).** `Esc` from the Monitor view closes
  the feed (`Command::CloseFeed`, from `m3-feed-connection.md`), not just switches `View` back to
  `Keys` while leaving the connection open in the background.
- **The render loop never does I/O.** The feed task (shell) produces `Msg::MonitorLine`s;
  `update()` only ever folds them into `MonitorState`.
- **Colors are semantic tokens.** The warning banner and any per-line highlighting (e.g. a command
  type color, if that's worth doing) use theme tokens.
- **Screen space is a budget, not a canvas.** Monitor is a `View`, not a pane addition.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/monitor.rs` (new) | `MonitorState`, `MonitorLine` |
| `crates/core/src/state/mod.rs` | `View::Monitor`; `pub monitor: MonitorState` on `State` |
| `crates/core/src/msg.rs` | `Msg::MonitorLine { at_ms, raw }` |
| `crates/core/src/update/monitor.rs` (new) | `push_monitor_line` (the one cap-enforcing function), pause/resume, filter capture |
| `crates/core/src/render/mod.rs` | Monitor screen: warning banner, tail, filter box, pause indicator |
| `crates/core/src/keymap/mod.rs` | `Action::OpenMonitor`, `Action::TogglePause` |
| `crates/app/src/redis/feed.rs` | `FeedKind::Monitor` handling (from `m3-feed-connection.md`) |

## Testing

- **Core unit tests**: `push_monitor_line` never grows `lines` past `cap` across a long synthetic
  sequence (the direct analogue of ADR-0010's million-key cap test, scaled to Monitor's cap); a
  paused `MonitorState` does not grow `lines` when fed lines, and increments
  `dropped_while_paused` instead; resuming does not backfill the dropped lines; the filter narrows
  the *rendered* subset without shrinking `lines` itself; `Esc` emits `Command::CloseFeed`.
- **Golden frames**: the warning banner present at every Density (it must survive down to
  DESIGN's 80-column floor, R7.1); the tail with lines, paused (with the drop count visible),
  filtered, and empty (feed just opened, nothing has arrived yet).
- **Docker-backed integration test**: open a real `MONITOR` connection against a live container
  (`m3-feed-connection.md`'s `open_feed`), issue a known command on a second ordinary connection,
  and prove the line arrives and is parsed; issue enough commands to exceed the test's cap and
  prove the buffer stays bounded against a real stream rather than only a synthetic one; prove a
  paused feed's underlying `MONITOR` connection is still alive and readable (not torn down) but
  `MonitorState.lines` does not grow across it — this is the test that actually proves "pausing
  stops consuming the feed, not just hides it," PLAN's stated proof for this row, against the real
  mechanism rather than a mock.

## CONTEXT.md

No glossary entry exists yet for **Monitor**. Proposed, to be added when this feature's
implementation actually starts:

**Monitor**:
A live, unfiltered-by-Redis tail of every command the server executes, driven by `MONITOR` on its
own dedicated connection (never the main one — see `m3-feed-connection.md`). Expensive by nature,
which is why the view carries a persistent warning rather than a dismissible one, and why its
buffer is capped rather than retained in full like the Loaded set.
_Avoid_: Log, command log, trace, tail (ambiguous with the app's own filtered display of it)

## Out of scope

- **Persisting Monitor output to a file** — not asked for by R6.1; the app's `y` copy machinery
  already covers "get this text out of the terminal" for a selection, and a file-export feature is
  a different surface.
- **Filtering server-side** (`MONITOR` itself has no filter argument) — the filter box is always a
  client-side narrowing of what already arrived, never a way to reduce what the server sends,
  which is exactly why the cost warning cannot be conditioned on "but I've filtered it down."
