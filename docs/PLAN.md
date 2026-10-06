# redis-pane — Implementation plan, M0 and M1

**Status:** Draft v0.1 · **Companion to:** [PRD.md](PRD.md), [DESIGN.md](DESIGN.md) ·
**Last updated:** 2026-08-26

This plan covers the two milestones that produce a usable product. M0 is an internal checkpoint —
it connects and browses nothing, so it has a test suite rather than users. M1 is the milestone
that already beats `redis-cli` for daily work.

Every task names what it *proves*. A task without a proof is a task that will be re-done.

## 1. Approach

**One vertical slice first.** M0 builds a single thin path end to end — resolve, connect, probe,
render one frame — before any breadth. The architecture is unusual enough (functional core,
injected clock, columnar arena, push liveness) that proving it end to end early is worth more
than completing any horizontal layer.

**Liveness transport lands in M0, not M1.** It is the riskiest assumption in the project:
capability probing, push handling, and re-arming. The transport was validated against Redis 8.4.0
before this plan was written — see the Verification table in
[ADR-0006](adr/0006-liveness-without-a-refresh-button.md), which confirmed the mechanism and
turned up one behaviour the design had not accounted for: tracking is consumed by its own
invalidation, so every Refetch must re-arm. M0 builds the connection layer anyway, so the
transport is proven there and M1 only adds the interface on top.

**Golden frames from the first screen.** The design mockups already produced during planning are
character grids generated deterministically from fixed state — structurally the same artifact as
a `TestBackend` snapshot. The suite starts as soon as anything renders.

## 2. Workspace layout

The core/shell boundary is enforced by the compiler, not by review
([ADR-0011](adr/0011-functional-core-golden-frames.md)). `redis-pane-core` does not depend on
`tokio`, `fred` or `crossterm`, so I/O in the render path is a compile error.

```
redis-pane/
├── Cargo.toml                 workspace
├── crates/
│   ├── core/                  redis-pane-core  — pure
│   │   ├── state/             App state, Loaded set arena, Viewer state, Connection state
│   │   ├── msg.rs             Msg — everything that can happen
│   │   ├── command.rs         Command — everything the shells must do
│   │   ├── update/            update(State, Msg) -> (State, Vec<Command>)
│   │   │   ├── mod.rs          update(), key_press(), Mode + mode(), read helpers
│   │   │   ├── link.rs         connection, tracking and server-condition messages
│   │   │   ├── scan.rs         scan and metadata batches; the Loaded set cap
│   │   │   ├── keys.rs         selection, filter capture, tree fold, sort
│   │   │   ├── viewer.rs       value replies, cursor movement and scroll, copy
│   │   │   ├── editor.rs       inline edits: open, type, stage
│   │   │   ├── confirm.rs      the dialog, and what a settled mutation does
│   │   │   └── mouse.rs        clicks, scroll, divider drags
│   │   ├── render/            panes, Viewers, title bar, hint bar
│   │   ├── theme/             semantic tokens, palettes, capability degradation
│   │   ├── keymap/            bindings as data
│   │   ├── config/            schema types + validation
│   │   ├── resolve.rs          flags -> Profile -> environment -> localhost
│   │   └── clock.rs           Clock trait
│   └── app/                   redis-pane — shells
│       ├── main.rs            args, resolution entry, startup diagnostics, exit codes
│       ├── terminal.rs        crossterm, raw mode, event loop, resize
│       ├── redis/             fred client, executor, capability probe, tracking
│       ├── config_io.rs       file read, permission refusal
│       └── state_file.rs      $XDG_STATE_HOME persistence
└── tests/                     integration, testcontainers
```

`ratatui` belongs in core: it draws into a buffer and performs no I/O. Config *types and
validation* are core and pure; config *reading* is a shell.

## 3. M0 — Skeleton

Proves: the architecture holds, and the app can be trusted about what it is connected to.

**Progress: complete.** The boundary is enforced by CI, the clock is injected, resolution and
config parsing carry their full test tables, the Redis shell connects over RESP3 and probes for
`CLIENT TRACKING`, both re-arm invariants are asserted, and every readout in DESIGN §6.9 is a
recorded golden frame.

| # | Task | Proves |
|---|---|---|
| 1 | Workspace scaffold, CI running `fmt`, `clippy -D warnings`, `test` | The boundary compiles; core has no I/O deps |
| 2 | `Clock` trait, injected everywhere | Two renders at different wall-clock times produce identical frames |
| 3 | Theme tokens + truecolor / 256 / monochrome degradation | Golden frames of one screen in all three modes |
| 4 | `Msg`, `State`, `Command`, `update` skeleton | A state transition test with no runtime attached |
| 5 | Terminal shell: raw mode, event loop, resize, quit | Synthetic events drive `update` without a terminal |
| 6 | Connection resolution chain, flags → Profile → env → localhost, carrying **Source** | Table-driven tests over the whole precedence matrix (R1.2, ADR-0001) |
| 7 | Config schema, strict parse rejecting unknown fields, permission refusal | Line/column errors; a typo'd `passwordEnv` fails loudly; group-readable file refused (R1.5, R1.7) |
| 8 | Redis shell: `fred`, RESP3, version floor, **capability probe** | Connects to 6.2 and 7.x; Redis 5 is refused with a diagnostic rather than a protocol error; a server refusing `CLIENT TRACKING` degrades to `○ manual` (R1.13, ADR-0007) |
| 9 | Startup diagnostics and exit codes | Unreachable target exits non-zero with target, Source and cause on stderr (R1.14) |
| 10 | Reconnect with visible backoff, both re-arm invariants | The shell redials on `Command::Reconnect` (exponential backoff, `redis::backoff_for`), redoing the full startup ritual — version floor, tracking probe, server conditions — since a server that vanished and came back is not guaranteed to still be the same one. `r` retries immediately rather than waiting out the timer (ADR-0009), cancelling whatever backoff or in-flight attempt was already running. Both re-arm invariants hold and are tested: the header never reads `● live` until tracking is armed, and a second write after an invalidation produces a push only once the Refetch re-armed (ADR-0006, ADR-0009). The header states the retry countdown while one is scheduled |
| 11 | Title bar: Environment dot, target, db, Source, Read-only reason | Golden frames of every readout in DESIGN §6.9, including `replica … locked` |
| 12 | Help overlay, keymap as data, hint bar | An overridden binding changes the on-screen hint (R7.5) |

**Done when** `redis-pane staging` opens against a real server, shows exactly what it is
connected to and why, survives the server being restarted underneath it, and exits usefully when
it cannot connect. No keyspace. — **Met.** A dropped link keeps the last read value, says
`✕ disconnected`, states the Read age, and reconnects on its own with a visible backoff
(M0.10); `r` retries immediately rather than waiting out the timer.

## 4. M1 — Browse

Proves: the keyspace is legible, the values keep their shape, and the screen is never lying about
how current it is.

**Progress: complete.** The Loaded set holds a million keys in 40MB, the keyspace source
streams and cancels against a real server, the browser renders at every breakpoint with metadata
filling in behind placeholders that hold their column, filter, tree and sort all work by
permuting an index vector rather than touching the arena, and every Redis type renders behind one
shared frame whose liveness states are recorded as golden frames.

| # | Task | Proves |
|---|---|---|
| 1 | Columnar Loaded set: byte arena + parallel metadata arrays, hard cap | 1M synthetic keys inside the memory budget; cap stops scanning and says so (R2.6, ADR-0010) |
| 2 | Keyspace source: a stream of keys with progress, `SCAN` driver behind it | 100k-key scan streams, resumes, and cancels on `Esc`; the abstraction hides the cursor count (R2.1, ADR-0008) |
| 3 | Virtualized key list, four columns, responsive breakpoints | A golden frame in each band of DESIGN §2 — 130, 119, 100, 80, 70, 60 — and a frame drawn from a 200k-key Loaded set in under 16ms (R2.4, R2.6, R7.1) |
| 4 | Lazy metadata fetch with placeholders | Pending cells render without shifting layout, and only the visible window is ever fetched — three pipelined commands per row, not per keyspace (R2.4) |
| 5 | Filter: glob and fuzzy over the Loaded set | Unit tests plus a golden frame |
| 6 | Tree / flat toggle, prefix index over the same arena | No second copy of the key names (R2.3) |
| 7 | Sort across the Loaded set | Sorting a lazily-fetched column orders what arrived, parks the rest, states the count — and issues no mass fetch (R2.5) |
| 8 | `Viewer` trait and shared frame — header, body, footer | Navigation transfers between types because only the body changes (R3.1) |
| 9 | Type Viewers: string, hash, list, set, zset, stream, JSON, binary | A golden frame per type against a testcontainer fixture |
| 10 | Liveness UI on M0's transport: arm on open, apply / announce / held, deleted retention | Golden frames of all seven header states; a key modified externally lands without a keypress (R3.6–R3.11) |
| 11 | Local TTL countdown | Injected-clock test; no round trip |
| 12 | `y` copy — key, value, `redis-cli` command | Payloads are exact, the command matches the key's type, and OSC 52 puts it on the *local* clipboard over SSH |

**Done when** a 100k-key keyspace is browsable in under a second, every type renders as itself,
and a key changing on the server updates on screen without anyone pressing anything. — **Met.**

## 5. M2 — Mutate

Proves: a mutation can be trusted — the reader always sees the real command and its blast radius
before it runs, Read-only Mode is enforced at exactly one point, and nothing here reopens the
RedisInsight-shaped bugs M0/M1 were built to make structurally impossible.

**Progress: in flight.** Task 1 was already done incidentally while building M0's title bar and
Ctrl-R toggle — `ReadOnlyReason`, its precedence rules, and the DESIGN §6.9 chrome all shipped
with golden-frame coverage before this table existed. Tasks 2–4 (the chokepoint, Delete, and
String edit) are done. The rest is ordered per a grilling session with the user: value edit ships
type by type (String → Hash → Set → List → ZSet) before TTL editing, single-key confirmation is
always one keypress regardless of Environment (friction scales with count, not Environment — that
is Read-only Mode's job), and move-across-db (part of R4.3) is parked — see the note below.

Task 4 landed differently twice. First, after a grilling session, as String edit shelling out to
`$EDITOR` on a temp file rather than an inline text buffer, on the reasoning that no terminal
reliably distinguishes "commit" from "insert a newline" for a hand-rolled multi-line editor. That
needed its own fix first — the input-reading thread blocked on `crossterm::event::read()` forever,
which would have raced a child editor process for the same terminal input, so it briefly became a
pausable poll loop — and landing it caught a second real fred footgun (see
`crates/app/src/redis/mod.rs`'s `set_value` doc comment) along the way.

Manual testing over a real terminal then found a live bug in that approach: vim's own
terminal-colour query (`ESC]10;?`/`ESC]11;?`) can reply *after* the editor has exited, land on our
input thread, and replay as a burst of keystrokes — discarding the confirm dialog and typing into
the filter. See [ADR-0014](adr/0014-values-are-edited-inline.md) for the root cause. Task 4 was
reworked a second time to an embedded editor (`ratatui-textarea`) directly in the value pane,
which removes the terminal handoff — and the race in it — from the default path; `$EDITOR`
survives as a separate, later, opt-in escape hatch (task 5 below) rather than the default.

| # | Task | Proves |
|---|---|---|
| 1 | Read-only Mode as real state: `State` field, reason (`environment`/`replica`/`user`), `Ctrl-R` toggle, title-bar chrome | Golden frames of all four DESIGN §6.9 readouts; `replica` is never liftable (R4.5, ADR-0004) — **done** |
| 2 | Mutation chokepoint: `PendingMutation`, `State::confirm`, propose → preview → confirm → execute as `Command`/`Msg` additions | A state-transition test proves Read-only Mode refuses *at confirm*, after composing the real command, never at the keypress that staged it (R4.4, DESIGN §6.5) — **done** |
| 3 | Delete: single key, `DEL <key>`, preview + one keypress (`d` stages, `y` confirms, `Esc` dismisses) | Confirmed and refused paths both covered by unit tests; a completed delete reuses the existing "gone" badge machinery (`state.keys.set_gone`), so a deleted row behaves exactly like one that expired or was evicted — **done** |
| 4 | Value edit — String (and JSON-as-string, R3.2): `e` opens an inline editor in the value pane; `Ctrl-S` stages a `SET` with a stacked red/green diff preview; values over 200KB are refused with a notice (ADR-0014) | Core unit tests cover open/stage/discard, the JSON-invalid indicator, the size refusal at the boundary, and that a live update is held while the buffer is open (R3.8); golden frames pin the editor body, the wrapped-line case, and the hint bar; a Docker-backed integration test proves `SET` actually lands — **done** |
| 5 | `$EDITOR` escape hatch: `E` in the value pane and `Ctrl-E` inside the inline editor seed `$VISUAL`/`$EDITOR`/`vi` with the current buffer, hardened against the terminal-reply race ADR-0014 documents (synchronous pause handoff, a settle delay, an input-buffer flush before returning to raw mode) | An app-level test fakes `$EDITOR` with a shell script (no Docker) to prove the temp-file round trip, the trailing-newline normalization, and that a leaked terminal reply after the settle window is drained rather than replayed as keystrokes |
| 6 | Value edit — Hash: field/value inline edit, add/remove field, one field at a time, via a guarded Lua script rather than a plain `HSET`/`HSETNX` (ADR-0015). Follow-up: `e`/`a`/`d` act on the Open key only with the value pane focused and its value fetched (G), and adding a field is a two-part `FIELD`/`VALUE` form with a live shown-duplicate guard, replacing the one-line name capture | Docker-backed tests prove the edit script keeps a field's own TTL (7.4+) and still runs on 6.2 where `HPEXPIRETIME` doesn't exist, never recreates a gone key, and never overwrites an existing field on add; core unit tests cover focus gating for `e`/`a` (including the refusal notices), the add form's name/value parts and the shown-duplicate block, the three confirm dialogs (including the last-field warning), read-only refusal, and R3.8 held-while-editing for a field; golden frames pin the add form's name/value/duplicate states, the read-only-FIELD edit state, all three dialogs and the Hash hint bar — **done**, ADR-0015 |
| 7 | Value edit — Set: add/remove member, one at a time, via a guarded Lua script for add rather than a plain `SADD` (ADR-0016); no in-place edit — a changed member is a rename, deferred to task 14 alongside Hash field rename | Docker-backed tests prove the add script never recreates a gone key and never duplicates an existing member (including one outside the 500-member read window), that `SREM` removes the last member and the key goes with it, that a binary member round-trips, and that `WRONGTYPE` on a key of another type surfaces as an error rather than a panic; core unit tests cover focus gating for `e` (refusing with D1's notice)/`a`/`d`, the single-part add form and its shown-duplicate block, both confirm dialogs (including the last-member warning), read-only refusal, and R3.8 held-while-editing for a Set; golden frames pin the add form's empty/typing/duplicate states, both dialogs and the Set hint bar — **done**, ADR-0016 |
| 8 | Value edit — List: index-addressed edit, add at either end, remove, via a guarded Lua compare-and-set on the element rather than a plain `LSET`/`LREM` (ADR-0017); insert *between* existing elements is out of scope — `LINSERT`'s pivot is addressed by value and matches the first occurrence, so it cannot address a position on a list with duplicates, and is deferred to its own row | Docker-backed tests prove an edit lands, an edit refuses without writing when the list shifts under it (a concurrent push between read and write — the test this task exists for), an edit refuses on a gone key and does not recreate it, an add refuses on a gone key and does not recreate it, a delete removes the *right* element on a list with duplicate values (not `LREM`'s first-match-from-head), a delete of the last element takes the key with it, a binary element round-trips, and `WRONGTYPE` surfaces as an error rather than a panic; core unit tests cover focus gating for `e`/`a`/`d` (including the binary refusal), the add form's head/tail toggle, all three confirm dialogs (including the last-element warning and `NotWritten::ElementMoved`'s wording), read-only refusal, R3.8 held-while-editing for a List, and that a row past the 500-item read window cannot be staged; golden frames pin the element editor, the add form in both `Head` and `Tail` states, all three dialogs and the List hint bar — **done**, ADR-0017 |
| 9 | Value edit — ZSet: edit member's score, add/remove member+score, via a guarded Lua script for both the score edit and the add rather than a plain `ZADD` (ADR-0018); no in-place member edit — a changed member is a rename, deferred to task 14 alongside Hash field rename and Set member rename | Docker-backed tests prove a score edit lands and keeps the key's TTL, refuses on a gone key without recreating it, refuses on a gone member, and lands on the right member after the set has been reordered under it (D7 — rank is not identity, so it cannot be misdirected the way a List index can); that an add refuses on a gone key without recreating it and refuses a duplicate member without changing its score; that removing the last member takes the key with it; that a binary member's score round-trips; that `inf`/`-inf` round-trip; that a high-precision score round-trips byte-identically; and that `WRONGTYPE` surfaces as an error rather than a panic; core unit tests cover focus gating for `e`/`a`/`d`, D5's binary-member score edit, D4's numeric validation (including `inf`, `-inf`, `nan` and garbage), the add form's `MEMBER`/`SCORE` parts and its shown-duplicate block, all three confirm dialogs (including the last-member warning and `NotWritten::MemberGone`'s wording), read-only refusal, R3.8 held-while-editing for a ZSet, and a long-ZSet case past the 500-member read window; golden frames pin the score editor, the add form in both parts plus its duplicate and invalid-score states, all three dialogs and the ZSet hint bar, and prove the score-edit dialog visibly differs from the add/remove dialogs — preview shows a score diff distinctly from a membership diff — **done**, ADR-0018 |
| 10 | TTL editing: set, persist, extend and shorten, one field resolving to a plain `EXPIRE` (set) or one of two guarded Lua scripts (persist; extend/shorten), via `t` focus-split against `ToggleTree` (ADR-0019) | Docker-backed tests prove a set lands and reads back via `TTL`, a set on a gone key returns `KeyGone` without recreating it, a persist clears the expiry and settles `NothingToRemove` on a key that already had none, an extend/shorten applies against the server's TTL at write time rather than the one staged at open time (the reorder-analogue test D7 exists for), a shift on a key with no expiry refuses with `NoExpiry`, a shift past zero refuses with `WouldExpireNow` and leaves the key in place, and `EXPIRE` behaves identically on a String, a Hash and a ZSet — the first edit in this project that is not per-type at all (D1); core unit tests cover D4's grammar and every refusal by name, all three confirm dialogs, read-only refusal, and R3.8 held-while-editing; golden frames pin the capture, its live resolution line, all three dialogs and the `had no expiry` warning. Local countdown (M1.11) still ticks between reads with no round trip — that is the existing per-frame projection, not the write: the new TTL itself reaches the Viewer only through the post-write refetch (D7), never applied locally, and `PERSIST` clears it (R4.2) — **done**, ADR-0019 |
| 11 | Rename: `RENAME`, collision handling when target key exists | Preview shows source → target; a colliding target is caught before execute, not as a server error surfacing after. **On a Cluster** (ADR-0022): the preview also warns when source and target hash to different slots, because `RENAME` across slots fails with `CROSSSLOT` |
| 12 | Copy: `COPY`, collision handling | Same as Rename; TTL carries over per Redis's own `COPY` semantics, not reimplemented. **On a Cluster** (ADR-0022): the same cross-slot warning, and a cross-slot copy falls back to `DUMP` then `RESTORE … REPLACE?` behind the same confirm dialog, with the TTL carried over explicitly (`RESTORE`'s TTL argument), since `COPY` refuses across slots. Move-across-databases is hidden on a Cluster, which has only db 0 |
| 13 | Bulk operations: multi-select (`Space`, not yet bound) feeding the same chokepoint; bulk delete with typed key-count confirmation on `prod` | Confirmation friction scales with count exactly as DESIGN §6.5 specifies; single-key path (task 3) is untouched — bulk is additive, not a rewrite. **On a Cluster** (ADR-0022): keys are grouped by node, each group deleted on its owner, and the `prod` typed confirmation counts every key across every node, never one node's share |
| 14 | Hash field rename: atomic `HSETNX new` + `HDEL old` in one guarded script, follow-up to task 6's D3 | Preview shows old name → new name; refuses the same way task 6's add does when the new name is already taken, and the same way its edit does when the key is gone |

**Task 6 ran before task 5**, by the user's choice on 2026-09-11 — the `$EDITOR` escape hatch
stayed parked so the Hash field work (higher-value, and needed regardless of whether the escape
hatch ships) could go first.

**Parked, not in this pass:** move-across-db (half of R4.3). Redis `MOVE key db` runs over the
*same* connection to a different numeric db index — it never opens a second connection or lets
the UI browse the destination, so it does not reintroduce the switcher ADR-0005 rejected — but
there is no daily-use need for it right now. Revisit after the first M2 RC ships. Undo/redo is
not planned anywhere in M2: the command-preview step (R4.4) is the stated safety net, not a
history to revert.

## 6. M3 — Power

Proves: the app reaches past a single key without giving up the shape everything before M3 held
to — one Connection, two panes (R7.7, ADR-0005). Every bound action stays reachable without
hunting for a keybinding, and the server's own operational signals — slow commands, the command
stream, pub/sub traffic, its own vitals — are visible without leaving the terminal or opening a
second one.

**Progress: done.** All six tasks are built. Task 1 (Palette) shipped in 0.1.0-alpha.14, then was withdrawn
(ADR-0020): `Ctrl-K` opened a fuzzy list over every `Action` in `crates/core/src/keymap/mod.rs`,
reading the same `Keymap` the hint bar and help overlay already read rather than a second copy of
it. Hands-on testing found no use for it — every `Action` it listed already had a key, so it was
only ever a slower route to the same thing — and it was removed completely. Discovery for the
occasional on-call user moves to a contextual help overlay (`?`/`F1`, R7.5), built as its own
change — [`m3-contextual-help.md`](plans/m3-contextual-help.md). One piece of the task's own structure survives the
withdrawal: `key_press`'s old inline `match action { … }` stayed factored out as
`update::dispatch_action`, since the keymap path is still its caller. Task 3 (Slowlog) is done,
pulled ahead of task 2 since it needs no feed connection — see
[`m3-slowlog.md`](plans/m3-slowlog.md) for the full build, including the `View`/`State.screen`
state machine and the `g`-prefixed chord machinery Dashboard/Monitor/Pub-Sub will reuse. Tasks 2
and 4 are done together, in one PR — [`m3-feed-connection.md`](plans/m3-feed-connection.md)'s own
framing ("plumbing with no consumer to drive it by hand") — phase A built the dedicated-connection
plumbing, phase B built Monitor on top of it. `fred::monitor::run` only accepts a Centralized
`ServerConfig`; Sentinel (a v1 target, ADR-0008) is handled by rewriting the feed's Config to
Centralized against the primary the main connection already resolved, rather than either failing
with fred's own generic Config error or leaving Sentinel silently unsupported — see
`crates/app/src/redis/feed.rs`'s `monitor_config`. Task 5 (Pub/Sub) is done — see
[`m3-pubsub.md`](plans/m3-pubsub.md) for the full build: a `SubscriberClient` on the same
feed-connection plumbing (no reconnect policy; Sentinel needs no rewrite the way Monitor's does,
since it is an ordinary client), a shared `state/tail.rs` `LiveTail<T>` Monitor was refactored onto
so the two live tails share one cap/pause/following/filter implementation, and every `FeedToken`
minted from one counter shared across features rather than one per feature. Task 6 (Dashboard) is
done — see [`m3-dashboard.md`](plans/m3-dashboard.md): the re-decision point at the top of that
plan was re-asked once the Slowlog had shipped, and the answer was to build it. `INFO` is
request/response on the main connection like `SLOWLOG GET`, polled by a shell-side
`tokio::time::interval` that always ticks; the core decides from state alone whether a given tick
fetches (`Msg::DashboardPollTick`), and every fetch carries a core-minted token so a manual `g d`/`r`
overlapping a timer poll cannot let a stale reply land over a newer one.

| # | Task | Proves |
|---|---|---|
| 1 | ~~Palette (`Ctrl-K`): fuzzy list over every app action, reading the same keymap-as-data source the hint bar uses (CLAUDE.md's "Keybindings are data")~~ | done in alpha.14, then withdrawn (ADR-0020) |
| 2 | ~~Dedicated-connection plumbing for push/poll feeds: a second `fred::Client` (or equivalent) the shell can hand to Monitor/Pub-Sub without starving the main read/write path~~ | done — proven both by the plumbing's own integration tests and by Monitor (task 4) actually using it: an ordinary read on the main connection completes without waiting on an open feed; a killed feed connection surfaces `Msg::FeedClosed` rather than hanging; `Command::CloseFeed` really disconnects |
| 3 | ~~Slowlog viewer (`g s`): `SLOWLOG GET`/`RESET`, sort, single-node (ADR-0008)~~ | done — entries render in a type-aware-consistent frame; `RESET` is a real mutation (confirm dialog, read-only refusal including `replica`) |
| 4 | ~~Monitor (`g m`): live tail, filter box, pause/resume, bounded buffer with a visible cap, persistent cost-warning banner~~ | done — the buffer never grows past its cap over a long synthetic run and against a real stream; pausing stops consuming the feed (proven against a real connection, not a mock) rather than hiding it; the banner survives to the single-pane floor |
| 5 | ~~Pub/Sub (`g p`): subscribe to channels and patterns, live tail~~ | done — a two-column (channel, payload) tail plus a subscription-chip strip that Monitor has no analogue of; `Esc`/`g k`/`g s`/`g m`/quit all close the feed connection, proven against a real server's own `PUBSUB CHANNELS`/`NUMPAT` accounting, not just the app's own state |
| 6 | ~~Dashboard (`g d`): `INFO`-based tiles — memory used/peak/maxmemory bar, hit ratio, ops/sec sparkline, clients, replication role/lag, eviction/expiry counters, single-node (ADR-0008)~~ | done — alarming values are colored through the theme's semantic tokens; every tile expands to its raw `INFO` section as a scrollable overlay; refreshes on a shell-side interval the core decides whether to act on, not static; a core-minted token drops a stale reply behind a newer manual fetch or poll |

**Console (R5.2–R5.4) is explicitly out of scope for M3.** The Palette (R5.1) shipped and was
withdrawn (ADR-0020) — see [PRD.md §10](PRD.md) for the resolved open question and the rationale.

### 6.1 Individual plan docs

Each row above has its own doc under `docs/plans/`, in the shape M2's per-task docs used —
context, approach, files touched, tests — written before any of it is built:
[`m3-palette.md`](plans/m3-palette.md) (superseded by
[`m3-palette-withdrawn.md`](plans/m3-palette-withdrawn.md), then by
[`m3-contextual-help.md`](plans/m3-contextual-help.md)),
[`m3-feed-connection.md`](plans/m3-feed-connection.md) (task 2 — shared infrastructure Monitor
and Pub-Sub both depend on, so it is its own doc rather than duplicated in each),
[`m3-slowlog.md`](plans/m3-slowlog.md), [`m3-monitor.md`](plans/m3-monitor.md),
[`m3-pubsub.md`](plans/m3-pubsub.md), and [`m3-dashboard.md`](plans/m3-dashboard.md) — the last
opens with the descope alternative as a named decision point, not buried in prose.

## 7. M4 — Scale & polish

Proves: the app holds up past the sizes and shapes M0–M3 were built and tested against — a
million-key keyspace, a terminal that cannot show truecolor or Unicode, a session the reader
wants back exactly as they left it — without giving up anything M0–M3 settled. Cluster, which
PRD §9 previously listed under this milestone, does not ship here: a Cluster target quietly
misbehaves today (one arbitrary node's `SCAN`/`INFO`/tracking reported as the whole truth), and
the decision this milestone makes about it is to refuse it clearly rather than build it —
[ADR-0021](adr/0021-cluster-refused-until-supported.md). Real support is its own milestone, M5,
after the beta, designed in [`m5-cluster.md`](plans/m5-cluster.md). Rebuild cost at real `SCAN`
order (below) is likewise deferred, to M6, designed in [`m6-perf-rebuild.md`](plans/m6-perf-rebuild.md). M4 ends in a beta (`0.1.0-beta.1`), not
a 1.0.

**Progress: done.** All eight tasks are built, released as `0.1.0-beta.1`. Task 1 (Cluster refusal)
refuses a Cluster-scheme URL in `build_config` before any connection attempt, and a plain
`redis://` URL to a cluster-enabled server via the `cluster_enabled:1` check folded into the `INFO`
read `connect_with` already made; `infer_environment` now strips `-sentinel`/`-cluster` schemes, so
a loopback Sentinel URL resolves `local` — see [`m4-cluster-refusal.md`](plans/m4-cluster-refusal.md)
and ADR-0021. Task 2 (perf harness) added `strip = true` to `[profile.dist]` (8.21MiB, against the
20MB budget), a `size` CI job, `crates/core/tests/perf.rs` timing `update`+render at 1M keys, and a
Docker-backed timing test at 100k; baselines are in [`m4-perf-harness.md`](plans/m4-perf-harness.md).
Task 3 (million-key scan) made `scan_batch` extend the view incrementally in the flat scan-order
case and rebuild on a 25% geometric schedule when sorted or in tree mode, and coalesced redraws in
the shell to one per 16 ms; a whole 1M-key scan in tree mode fell from ~120 s to under a second —
see [`m4-perf-scan.md`](plans/m4-perf-scan.md). Task 4 (million-key interaction) narrows a filter
keystroke from the rows already shown (22.3 ms to ~0.3 ms), debounces a full rebuild by 100 ms,
removed per-key allocation from `Tree::rebuild`, gave `row_of` an inverse index, and moved metadata
dedup to a shell-side ledger with a core-minted epoch to drop stale replies — see
[`m4-perf-interaction.md`](plans/m4-perf-interaction.md). Task 5 (themes) made `Theme` data: `dark`,
`light` and `high-contrast` built-ins checked for WCAG AA by a test, user themes in config,
`--theme`, and `light`/`high-contrast` paint their own background — see
[`m4-themes.md`](plans/m4-themes.md). Task 6 (ASCII glyph fallback) put every glyph behind one set
with width-identical Unicode and ASCII variants, chosen by `--ascii`/`--unicode`, config `ascii`,
or the locale — see [`m4-glyphs.md`](plans/m4-glyphs.md). Task 7 (session restore) writes
`$XDG_STATE_HOME/redis-pane/state.json`, one entry per target, via a writer thread with atomic
temp-and-rename; a corrupt file is ignored with a notice — see
[`m4-session-restore.md`](plans/m4-session-restore.md). Task 8 (this close-out) bumped the version,
retitled `ALPHA.md` as beta notes (the filename is kept) and marked these docs done — see
[`m4-close-out.md`](plans/m4-close-out.md). Packaging stays the open end-of-alpha decision, recorded
in PRD §10.

**These task 2–4 figures are for keys loaded in name order, which is the easy case.** The perf
harness pushes `user:00000000`, `user:00000001`, … in order; real `SCAN` order is effectively
random, which defeats the sort and the tree fold's memory locality. Measured on 2026-10-04 at 1M
random-order keys, a tree toggle (sort plus fold) takes ~373 ms, or ~482 ms with deep names, against
~42 ms in the harness. Fixing that — an honest harness, a name order maintained as keys arrive, and
large rebuilds sliced across frames — is milestone M6, after the beta:
[`m6-perf-rebuild.md`](plans/m6-perf-rebuild.md).

Still unbuilt after M4 and not part of it: rename, copy, bulk operations and Hash field rename (M2
tasks 11–14, §5); Cluster support (M5); rebuild cost at real `SCAN` order (M6).

| # | Task | Proves |
|---|---|---|
| 1 | Refuse Cluster targets ([`m4-cluster-refusal.md`](plans/m4-cluster-refusal.md)) | A `redis-cluster://` URL or a Cluster-mode server (detected via `INFO cluster`'s `cluster_enabled:1`, so a plain `redis://` URL to a cluster node is caught too) fails at startup with a diagnostic naming ADR-0021, never a half-working session. `infer_environment` recognises `-sentinel`/`-cluster` schemes, so a loopback Sentinel URL is `local`, not `unknown`. Integration test against a real `cluster-enabled yes` container |
| 2 | Measure first ([`m4-perf-harness.md`](plans/m4-perf-harness.md)) | Each PRD §7 metric that can be measured has a repeatable check: release binary <20 MB as a CI step on the release profile; `update` + render at 1M keys within 16 ms (scroll, filter keystroke, sort); Loaded-set plus view memory at 1M keys inside 250 MB (core-side accounting); first `ScanBatch` <150 ms and interactive <1 s at 100k keys, as an integration timing test. The release profile gets `strip`. Baseline numbers are recorded in the doc before tasks 3–4, so the wins are measured, not claimed |
| 3 | Million-key scan ([`m4-perf-scan.md`](plans/m4-perf-scan.md)) | Scanning 1M keys does O(n) total list work, not O(n²): the view extends incrementally per page in scan order, and a full re-sort runs only when a non-scan sort or tree mode is active, deferred or batched. The shell coalesces redraws (one frame per tick, not per `Msg`). The `ttl_read_at` leak in `LoadedSet::clear` is fixed, with a regression test. Task 2's budgets pass at 1M |
| 4 | Million-key interaction ([`m4-perf-interaction.md`](plans/m4-perf-interaction.md)) | A filter keystroke at 1M keys meets the 16 ms frame budget: it narrows from the previous result when the query extends, and otherwise rebuilds once, debounced. The tree rebuild allocates no per-key `String` or `Vec`, and collapsed-group lookup is O(1). `row_of` uses an inverse index. Metadata fetches are deduplicated and the stale ones cancelled. Budgets pass |
| 5 | Themes ([`m4-themes.md`](plans/m4-themes.md)) | `Theme` becomes data: a token→colour palette per colour depth. Built-ins are dark (today's), light and high-contrast, all checked for WCAG AA contrast by a test. The theme is chosen by an additive config field `theme` and a `--theme` flag. User themes are token→colour maps in config, unknown tokens a parse error. Goldens exist for light and high-contrast (ADR-0011's promise). `NO_COLOR` treats an empty value as unset, per no-color.org |
| 6 | ASCII glyph fallback ([`m4-glyphs.md`](plans/m4-glyphs.md)) | Every hard-coded glyph (`✕ ⊘ ✎ ▌ ● ○ ⁎ ⟡ ▶`, sparkline and bar blocks, box drawing where needed) goes through one glyph set with a Unicode and an ASCII variant. A test enforces width-identical variants. ASCII is chosen by config or flag, or automatically when the locale isn't UTF-8. Goldens exist in ASCII mode |
| 7 | Session restore ([`m4-session-restore.md`](plans/m4-session-restore.md)) | Per ADR-0003, a state file at `$XDG_STATE_HOME/redis-pane/state.json`, keyed per target (no secrets), restores pane split, tree/flat, sort, filter and the selected key on relaunch. The app still never writes config. A corrupt or old file is ignored with a notice, never a crash. Writes are atomic (temp file plus rename) and happen on quit and on change, debounced |
| 8 | Close out M4 → beta ([`m4-close-out.md`](plans/m4-close-out.md)) | PLAN/PRD marked complete. `ALPHA.md` becomes beta notes (what to try covers themes, glyphs, restore). README status says beta. Release `0.1.0-beta.1`. Packaging remains the open end-of-alpha decision, recorded in PRD §10 |

**Risk order:**
1. Task 1, because it's a live correctness and safety gap today — a Cluster target already
   connects and silently serves one node's view as though it were the whole keyspace.
2. Task 2, before 3 and 4, so optimisations are proven against a measured baseline rather than
   claimed.
3. Task 5 before 6, because both reshape how render asks for presentation and task 6 reuses
   task 5's "presentation as data" pattern.
4. Task 7, which is independent of the rest and can land in any order relative to them.

### 7.1 Individual plan docs

Each row above has its own doc under `docs/plans/`, in the per-task shape M2 and M3 used —
context, decisions (with recommended defaults flagged to confirm at build time), architecture,
files touched, tests, the CLAUDE.md rules it binds, and what stays out of scope:
[`m4-cluster-refusal.md`](plans/m4-cluster-refusal.md), [`m4-perf-harness.md`](plans/m4-perf-harness.md),
[`m4-perf-scan.md`](plans/m4-perf-scan.md), [`m4-perf-interaction.md`](plans/m4-perf-interaction.md),
[`m4-themes.md`](plans/m4-themes.md), [`m4-glyphs.md`](plans/m4-glyphs.md),
[`m4-session-restore.md`](plans/m4-session-restore.md), and [`m4-close-out.md`](plans/m4-close-out.md).

## 8. Explicitly not in M0–M3

Palette, Console, dashboard, monitor, pub/sub, slowlog (M3) — Console cut, Palette shipped then
withdrawn (ADR-0020), see §6 above. Cluster is milestone M5, after the beta — designed in
[`m5-cluster.md`](plans/m5-cluster.md) and split into ten tasks in [`m5-planning.md`](plans/m5-planning.md). Rebuild cost at real `SCAN` order is milestone M6, also
after the beta — [`m6-perf-rebuild.md`](plans/m6-perf-rebuild.md); a Cluster target
is browsable from M5 task 5, the Dashboard shows the whole Cluster from task 7, and Slowlog and Monitor show a notice until task 8
(§7, [ADR-0022](adr/0022-cluster-supported-in-stages.md), which superseded the M4 refusal in
[ADR-0021](adr/0021-cluster-refused-until-supported.md)). Themes beyond the two M0 defaults, the ASCII glyph
fallback, session restore, and the million-key performance work are M4 (§7). Packaging and
distribution beyond the alpha's raw GitHub Release archives is a decision parked to the end of
alpha (PRD §9, §10) — M4 ships no official packages. Keybinding overrides from config are
explicitly **not** in M4 either, by the user's 2026-10-01 scope decision (see
[`m4-planning.md`](plans/m4-planning.md)). Keys-pane liveness, and the other open questions in
[DESIGN §9](DESIGN.md) — all decidable later without rework, which is why they are still open.

## 9. Risk order

The tasks most likely to invalidate something already decided, earliest first:

1. **M0.8 and M0.10 — capability probe and re-arm.** If tracking does not behave as ADR-0006
   assumes, the product's headline feature changes shape. Deliberately in M0.
2. **M1.1 — the columnar arena.** It shapes every access to the key list and is the most
   expensive thing here to retrofit.
3. **M1.2 — the keyspace source abstraction.** Cheap to build now, a rewrite once Cluster needs
   N cursors.
4. **M0.2 — the injected clock.** Trivial on day one; invasive after a hundred call sites render
   a TTL.
