# M4 task 7: Session restore

Status: **done.** See Outcome at the end for the build-time choices.

## Context

PLAN.md M4 row 7: "Session restore · Proves: per ADR-0003, a state file at
`$XDG_STATE_HOME/redis-pane/state.json`, keyed per target (no secrets), restores pane split,
tree/flat, sort, filter and the selected key on relaunch. The app still never writes config. A
corrupt or old file is ignored with a notice, never a crash. Writes are atomic (temp file plus
rename) and happen on quit and on change, debounced."

[ADR-0003](../adr/0003-app-never-writes-config.md) already settles the shape this task has to fit:
`config.json` is read-only from the app's perspective; "session state that the app *does*
persist — last Connection, pane sizes, active filter, scroll position — is written to a separate
state file under `$XDG_STATE_HOME/redis-pane/`, which is never hand-edited." DESIGN §7 repeats the
promise ("Persistent session state. Pane split, filter, and scroll position restore on relaunch,
keyed by target") and DESIGN §9's "Resolved since v0.6" note says explicitly that pane-split
persistence "needs the session-state file ADR-0003 describes, which does not exist yet either."

Confirmed by reading the code: **`crates/app/src/state_file.rs` does not exist** — `ls
crates/app/src/` has no such file, and `CLAUDE.md`'s own workspace layout diagram lists it as
planned (`state_file.rs — $XDG_STATE_HOME persistence`) but unbuilt. The one piece of state this
task restores that already exists in `State` today is `crates/core/src/state/mod.rs`'s
`split_adjust: i16` field, documented there as session-only (the offset nudged by `⌃←`/`⌃→`,
currently lost on every relaunch per DESIGN §9's note).

## Decisions

1. **File location and format**: `$XDG_STATE_HOME/redis-pane/state.json` (falling back the same
   way `config.json` falls back to `~/.config/...` when `$XDG_CONFIG_HOME` is unset — **confirm at
   build time** the exact fallback for `$XDG_STATE_HOME`; the XDG Base Directory spec's documented
   default is `~/.local/state`, which this app should follow for consistency with how it already
   handles `$XDG_CONFIG_HOME`'s fallback). JSON, matching `config.json`'s format, though this file
   is machine-written-and-read only — unlike `config.json` it is never hand-edited, so there is no
   requirement to keep it pretty or stable-ordered for a human reader, but doing so anyway costs
   nothing and helps debugging.
2. **Keyed per target.** The task brief and PLAN's row both say this: restoring staging's filter
   must never apply to a prod session opened later. **Confirm at build time** exactly what "target"
   means as a key — the resolved connection string is the obvious candidate, but it embeds whatever
   host/port/db was resolved, which could leak a hostname into the state file (not a secret, but
   worth being deliberate about) and means two different *Sources* resolving to the same literal
   target (e.g. a Profile vs. an ad-hoc URL that happen to point at the same host) would share
   state — probably the right behavior (same server, same session expectations) but worth stating
   as a deliberate choice, not an accident of implementation.
3. **What is restored, per target**: pane split (`split_adjust`), tree/flat mode, sort, filter
   text, the selected key name (bytes — Redis key names are not guaranteed valid UTF-8, R3.13's
   rule applied here too, so the stored form should be base64 or an equivalent byte-safe encoding,
   not a JSON string that would silently mangle a binary key name), and a last-seen-at timestamp
   (for display or staleness decisions, e.g. "restored from 3 days ago" — **confirm at build time**
   whether this is surfaced to the reader at all, or purely bookkeeping; DESIGN doesn't ask for a
   visible "restored" indicator, so the simplest default is silent restoration with no new chrome).
   **Never a password or any credential** — ADR-0003's and R1.6's existing boundary; target
   identity only.
4. **Restoring the selected key** happens after the first scan batch, locating the key by name in
   the freshly-scanned `LoadedSet`; if it never turns up (deleted since last session, or simply not
   yet scanned when the reader starts interacting), **restoration gives up silently** — no error,
   no notice, just no selection change from whatever the scan naturally produced. This mirrors the
   existing `relocate_open_key`/`row_of` machinery conceptually (find a key by identity after the
   rows have changed) but is a one-time startup action, not a per-rebuild one.
5. **A corrupt or old-version file is ignored with a one-time notice, never a crash.** The file
   format should carry an explicit version field from day one (even though this is the first
   version) so a future format change has something to check against — **confirm at build time**
   whether "old version" for this task means "a version field this binary doesn't recognize" (the
   only case that can exist on day one, if the schema ever changes later) or whether the plan should
   anticipate a concrete migration path now; the simplest correct choice for M4 is: unrecognized
   version or unparseable JSON → ignored, notice shown once (R7.4's non-blocking-notification
   mechanism, reused — this is exactly the kind of "something happened, the reader should know, but
   it must not block them" event that mechanism already exists for), session proceeds with no
   restored state, exactly as if the file did not exist.
6. **Writes are atomic**: write to a temp file in the same directory, then rename over the target —
   standard crash-safety pattern, avoiding a half-written file on a kill-mid-write. **Happens on
   quit and on change, debounced** — a per-keystroke write (e.g. on every filter character) would
   be wasteful and is exactly the kind of per-`Msg` I/O CLAUDE.md's "the render loop never does I/O"
   rule is adjacent to (the write itself is shell-side and off the render path either way, but a
   debounce still avoids needless disk I/O on every keystroke) — reuse the same debounce-timer
   shape `m4-perf-interaction.md` proposes for the filter-rebuild debounce, if that task lands
   first, or build it standalone otherwise (**confirm at build time** build order between this task
   and task 4, since PLAN's risk order lists task 7 as "independent" and does not require an
   ordering relative to 2–4).
7. **Core gets a pure `SessionState` snapshot/apply pair; file I/O stays in the shell.** Consistent
   with the functional-core rule applied everywhere else in this codebase: `crates/core` defines
   what session state *is* (a plain struct) and two pure functions — `State::session_snapshot(&self)
   -> SessionState` and something like `State::apply_session(&mut self, restored: SessionState)` —
   and `crates/app/src/state_file.rs` owns reading/writing the JSON file and calling those pure
   functions, the same split `config_io.rs` already has relative to `crates/core/src/config/`'s
   pure schema types.

## Architecture

Core: `crates/core/src/state/session.rs` (new) — `SessionState` (plain serializable-shaped struct;
note `redis-pane-core` has no `tokio`/`fred`/`crossterm` dependency per ADR-0011's boundary, but it
may already depend on `serde` for the config schema types, in which case this struct can derive the
same traits directly — **confirm at build time** whether `serde` is already a core dependency via
`config/mod.rs`, which the `deny_unknown_fields` attributes suggest it is), plus the
snapshot/apply pair on `State`. No new `Command`/`Msg` variants should be needed for the *read*
path if restoration is folded into existing startup sequencing (apply the restored `SessionState`
once, before or interleaved with the first scan, the same way the Open key's row is relocated after
scan results land) — but the *write* trigger (on quit, on debounced change) likely does need a new
`Command` (e.g. `Command::PersistSession`) the shell issues, keeping with "the render loop never
does I/O... every Redis work happens on async tasks that send messages" pattern extended to local
file I/O, which is the same non-Redis-but-still-shell-owned-I/O precedent `config_io.rs` and
`clipboard.rs` already set.

Shell: `crates/app/src/state_file.rs` (new, matching CLAUDE.md's own workspace-layout diagram) —
read at startup (before or alongside connection resolution — **confirm at build time** the exact
sequencing relative to scan start, since the selected-key restoration in decision 4 needs the scan
to have produced at least one batch first), atomic write on `Command::PersistSession` and on quit
(the shell's own exit path, not a `Command` — quit may bypass the normal `Command` dispatch loop,
so this needs its own direct call, **confirm at build time** exactly where the quit path lives in
`terminal.rs` relative to the loop this doc's sibling tasks are also touching).

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/session.rs` (new) | `SessionState`, `State::session_snapshot`/`apply_session` |
| `crates/core/src/state/mod.rs` | wiring for the snapshot/apply pair; possibly a `dirty`-since-last-persist flag feeding the debounce |
| `crates/core/src/command.rs` | `Command::PersistSession` (or equivalent) |
| `crates/app/src/state_file.rs` (new) | read-at-startup, atomic write, corrupt/old-version handling with a one-time notice |
| `crates/app/src/terminal.rs` | debounced persist trigger; write-on-quit |
| `crates/app/src/main.rs` | read the state file early enough to feed restoration into the startup sequence |

## Testing

- **Core unit tests**: `session_snapshot`/`apply_session` round-trip every restored field exactly;
  a binary (non-UTF-8) selected key name round-trips byte-identically through whatever encoding is
  chosen (decision 3); applying a `SessionState` whose selected key is absent from a fresh
  `LoadedSet` leaves selection at its natural post-scan default rather than erroring.
- **Shell/app-level tests** (no Docker needed — this is local file I/O, not Redis): a corrupt JSON
  file is ignored with the one-time notice and does not crash startup; a file with an unrecognized
  version field is ignored the same way; an atomic write survives a simulated interruption (write
  the temp file, do not rename, assert the original file — if any — is untouched); the debounce
  coalesces rapid changes into one write; quit always flushes a pending debounced write rather than
  losing it.
- **A by-hand script**, per the task brief's own verification note: relaunch against the same
  target after adjusting the split, filter, sort and tree mode, and confirm every one restores;
  relaunch against a *different* target and confirm none of it leaks across.

## CLAUDE.md rules this binds

- **Config is `~/.config/redis-pane/config.json`, and the app only ever reads it... Anything the
  app persists... goes to a separate state file under `$XDG_STATE_HOME/redis-pane/`.** This task is
  the direct build-out of that already-decided boundary; it must not touch `config.json` at any
  point, including on a code path that might be tempted to "save a profile" from a session (not
  asked for here, and ADR-0003 already rejected that shape generally).
- **The functional core, imperative shells** split: `SessionState` and its snapshot/apply functions
  are pure core; all file I/O is shell-side, in a new `state_file.rs` matching the module CLAUDE.md
  already names in its workspace-layout diagram.
- **Errors surface as non-blocking notifications** — the corrupt/old-file case is exactly this: a
  real failure (the file could not be used) that must not block startup and must not be silently
  swallowed, reusing R7.4's existing mechanism rather than inventing a second kind of notice.

## Out of scope

- **Any data beyond the five named fields** (pane split, tree/flat, sort, filter, selected key) —
  PLAN's row names exactly these; a broader "remember everything" session model is not asked for.
- **Cross-machine or cross-user sync of session state** — purely local, `$XDG_STATE_HOME`-scoped,
  same as every other local-only piece of state this app keeps.
- **A visible "restored from your last session" UI affordance** — decision 3 above defaults to
  silent restoration; adding chrome for it is a DESIGN-level decision not asked for by PLAN's row
  and would cost a line of permanent screen space per G7's "nothing on screen you will not act on."

## Outcome

Built as planned. The "confirm at build time" items resolved to the plan's recommended defaults:

- **`$XDG_STATE_HOME` fallback**: `~/.local/state/redis-pane/state.json` (the XDG default); an empty
  `$XDG_STATE_HOME` counts as unset, as `$XDG_CONFIG_HOME` does.
- **Target key**: `Connection::target` (`host:port/db`, the redacted display string — no credentials).
  Two Sources resolving to the same literal target deliberately share a session. The hostname does
  appear in the file; it is not a secret. The file holds at most 32 targets (`MAX_TARGETS`), evicting
  the least recently saved.
- **`saved_at_ms`**: stored for eviction only; not surfaced. Restoration is silent, no new chrome.
- **Selected key**: hex-encoded bytes in JSON. Restored by `scan_batch` when the key arrives (one
  extra `rebuild_list`, once); abandoned silently if the cursor has moved off row 0 first, if the
  key is filtered out, or when the scan ends without it. Until resolved, `session_snapshot` carries
  the pending key so an early write cannot overwrite it with row 0.
- **Version**: `version: 1`. Anything else, unparseable JSON, or an unknown sort/filter-mode string
  is ignored; no migration path is anticipated. The notice is an error notification (it persists
  until `Esc`, per R7.4) rather than a fading notice, which would be gone before the reader saw it.
  A later write replaces the ignored file.
- **Write trigger**: no `Command::PersistSession`. The shell observes `State::session_snapshot()`
  after every `update`; a change starts a 1s debounce (reset by further changes) and the pending
  snapshot is handed to a dedicated writer thread. Quit flushes and joins the writer after the
  terminal is restored; a failed final write is printed to stderr. Mid-session write failures are
  `Msg::Failed` notifications, once per outage. This kept the core free of a dirty flag and a
  command and did not touch the `Command` match in the shell. Independent of task 4's debounce.
- **Atomicity**: temp file (`state.json.tmp.<pid>`, mode `0600`, fsynced) renamed over the target;
  the file is re-read and merged on every write so another terminal's targets survive.
- **Core vs shell**: format, snapshot and apply are in `crates/core/src/state/session.rs`
  (`serde` was already a core dependency); I/O is in `crates/app/src/state_file.rs`. `main.rs`
  loads and applies before `terminal::run`, which gained a `session_store` parameter.

Tests: core round-trip, binary key, versioning/garbage, eviction, selection restore in flat, tree and
never-found cases; shell tests for path, per-target isolation, corrupt/old file notices, interrupted
write, `0600`, debounce coalescing, quit flush, and write-failure reporting. Not covered by an
automated test: the by-hand relaunch check (adjust split, filter, sort and tree mode; relaunch; then
relaunch against a different target).
