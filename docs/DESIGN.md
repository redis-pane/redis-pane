# redis-pane — UX & UI Design

**Status:** Draft v0.4 · **Companion to:** [PRD.md](PRD.md) · **Last updated:** 2026-09-27

## 1. Design principles

1. **The terminal is a canvas, not a constraint.** We use space, color, weight, and motion the
   way a well-made desktop app does. If something looks like ASCII-art compromise, it is a bug.
2. **Show, don't make them remember.** Contextual key hints are always on screen. The user
   should never need to recall a command to make progress.
3. **One primary action per pane.** Each pane has an obvious next step. Ambiguous focus is a
   design failure.
4. **Data keeps its shape.** A hash is a table. A stream is a timeline. Nothing is flattened
   into a string just because the terminal is text.
5. **Danger is visible before it is possible.** Environment color, read-only badge, and command
   preview all appear *before* the user commits.
6. **Never freeze.** Every network operation is cancellable and every long list is virtualized.
   A spinner is acceptable; a locked keyboard is not.
7. **Familiar to two tribes.** Vim keys and arrow keys both work, always. No modal purity tests.

## 2. Layout

Two columns, one status bar, one hint bar. The split is resizable (`⌃←`/`⌃→`, or a drag once
R7.3's mouse support lands); nothing else is permanent.

```
┌─ redis-pane ─ ● staging · cache-01:6379/0 · from profile ──────────────────┐
│ KEYS   scanning 41,203 of ~180,000      │ user:8812:session                │
│ / user:*:session            3,410 match │ hash · 14 fields · 2.1 KB        │
│ KEY                 TYPE      SIZE  TTL │ ttl 00:42:17                     │
│ ▾ user:                          41,203 │                                  │
│   ▾ 8812:                             6 │ FIELD          VALUE             │
│     ● session       hash    2.1 KB  42m │ id             8812              │
│     ● profile       json     880 B    ∞ │ device         ios/17.2          │
│     ● cart          zset     412 B  12m │ region         eu-west-1         │
│   ▸ 8813:                             6 │ cart_total     4                 │
│ ▾ cart:                           8,120 │ plan           pro               │
│   ● 91af3c9d…       zset    1.1 KB  12m │ locale         fr-FR             │
│   ● 91af41e0…       zset     380 B  58m │ exp_bucket     B                 │
│ ▸ feed:                           2,088 │ cart_rev       18                │
├─────────────────────────────────────────┼──────────────────────────────────┤
│ ↑↓ move  → open  / filter  d d delete   │ e edit  y copy  t ttl            │
└─────────────────────────────────────────┴──────────────────────────────────┘
                                                                              
 Esc back   ? help                             SCAN 23% ▓▓▓░░░░░  Esc cancel 
```

**There is no sidebar.** An earlier draft gave one to Profiles, live Connections and databases.
All three turned out to be launch-time concerns: the target is chosen by flag or Profile before
the process starts, and a second target means a second terminal
([ADR-0005](adr/0005-one-connection-per-process.md)). Sixteen permanent columns were showing
information the title bar already carried and offering switches nobody makes mid-session. They
now belong to the keyspace.

The title bar still carries all four things it must always carry: Environment dot, target,
database, and **Source** (`from profile` / `from --url` / `from REDIS_URL` / `default`). Per
[ADR-0001](adr/0001-connection-resolution-order.md) the app resolves silently, and this readout
is the entire mitigation for doing so — it is not optional chrome.

**Responsive behavior**

| Width | Layout |
|---|---|
| ≥ 120 cols | Two columns as above; the value pane takes the larger share |
| 90–119 | Two columns; the keys pane sheds `SIZE`, keeping `TYPE` and `TTL` |
| 70–89 | Two columns, tight; the title bar truncates the target from the left, never the Environment or Source |
| < 70 | Single pane, stack-navigated; breadcrumb replaces columns |
| Height < 24 | Hint bar collapses into the status bar |

`TTL` is the last metadata column to go because it is the field people are hunting when the
terminal is small and the situation is urgent.

**The Dashboard's own grid (`g d`, §6.6, M3 task 6).** A tile grid is a genuinely different
layout shape from the two-pane split above — it gets its own breakpoints, not the two-pane numbers
above force-fitted to it:

| Width | Tiles per row |
|---|---|
| ≥ 120 cols | 3 |
| 80–119 | 2 |
| < 80 | 1, and the grid scrolls |

Below 80 columns the six tiles (memory, hit ratio, ops/sec, clients, replication, eviction) do not
all fit vertically in a typical terminal height, so the grid scrolls with tile focus
(`←→↑↓`/`hjkl`) the same way the keys pane's own viewport follows the selection — stateless,
recomputed each frame from which tile is focused, never a persisted scroll position of its own.

## 3. Navigation model

- **`Tab` toggles the two panes**; focus is shown by border color *and* a
  brightened title — never by border alone (colorblind and monochrome safety).
  Focus alone never changes what `↑↓`/`j k` do, though — see `Enter` below.
- **In the key list**, `↑↓` / `j k` move, `→` / `l` descends (opens a key,
  expands a collapsed tree group, or steps into an already-expanded one),
  `←` / `h` ascends (collapses an expanded group, or moves to its parent).
- **`Enter` opens the Selected key and starts moving a real cursor inside
  it** — a highlighted row, not just a scroll position — and `↑↓` / `j k` /
  `PgUp` / `PgDn` / `Home` / `End` then move it instead of the key list,
  exactly the same keys, redirected. `Esc` exits back to the key list without
  closing the value. This is deliberately a separate keypress from
  `Tab`/focus: merely looking at the value pane must never silently
  reprogram what the movement keys do. A no-op on a tree group row — a group
  has no value of its own; `→` is what expands or steps into one.
  If the Selected key is already open and at rest, the cursor drops straight
  in with no read; otherwise `Enter` opens it first, same as `→`, and the
  cursor activates the moment that read lands — one keystroke to both dive
  into a key and start moving through it. This also means the value pane
  never shows a value it can be moved through only to leave it silently
  stale: switching the Selected key with the cursor already active (`Esc`,
  then `↑↓` to a different row, then `Enter` again) always re-anchors on
  what is actually selected rather than the last key `Enter` was pressed on.
- **Global jumps** use a `g`-prefixed chord: `g k` keys, `g d` dashboard, `g m` monitor,
  `g p` pub/sub, `g s` slowlog. There is no `g c` — there is only ever one Connection.
  Implemented as keymap data, not a hard-coded key sequence
  (`crates/core/src/keymap/mod.rs`'s `Keymap.chords: Vec<ChordBinding>`, beside the single-key
  `bindings`): `g` arms a pending prefix (`State.pending_chord`) and waits, with **no timeout**
  (§8: "no timing-dependent interactions"), for exactly one more keypress. `Esc`, or a second key
  that names no chord, clears the pending prefix and does
  nothing else — swallowed, the same way a stray keystroke is swallowed everywhere else in this
  app rather than reinterpreted. While a chord is pending, the hint bar and the help overlay show
  only its continuations (`k keys`, `s slowlog`) plus `Esc cancel` and the help key — not the
  ordinary global row list, which would otherwise repeat `g s slowlog` right after `s slowlog` in
  the same breath.
- **A full-screen view displaces both panes** (R7.7, G7) rather than living beside them —
  `State.screen: View` (`Keys` — the two-pane browser, default — or `Slowlog`, with `Dashboard`/
  `Monitor`/`PubSub` to follow their own tasks). Named `screen`, not `view`: `State.view` already
  names the keys pane's own scroll position (`crate::render::keys::Viewport`). `Pane` is unrelated
  and unaffected — it still decides keys-vs-value focus *within* `View::Keys`; `View` decides which
  full-screen surface is showing at all. Switching views (`g s`/`g k`/`Esc`) touches nothing else:
  the browser's own state (the Open key, its scroll position, its filter, tree mode) is exactly as
  it was left, and tracking on the Open key is never disarmed by leaving or returning to `Keys`.
- **The console** (`:`) is for raw Redis commands sent straight to the server. It is not
  scheduled — R5.2–R5.4 stay a real requirement, just not built (PRD §10).
- **`Esc` is always "back"**, and never destroys unsaved input without asking.
  It pops one thing at a time: the value cursor first if it is active, then
  a full-screen view back to the browser, then the pane stack, then an unrelated background
  operation.

## 4. Core keymap

| Key | Action | Scope |
|---|---|---|
| `?` / `F1` | Contextual help overlay (`F1` also works while typing) | global |
| `Tab` | Move focus between the panes | global |
| `g k` | Jump to the Keys view (the two-pane browser) | global |
| `g s` | Jump to the Slowlog view, fetching a fresh `SLOWLOG GET` | global |
| `g m` | Jump to the Monitor view, opening a `MONITOR` feed on its own connection (confirmed first on `prod`/`unknown`) | global |
| `g p` | Jump to the Pub/Sub view — dials lazily (nothing until the first subscription), no confirmation | global |
| `g d` | Jump to the Dashboard view, fetching `INFO` immediately and polling it every 2s while shown | global |
| `Ctrl-C` ×2 | Quit (single press = cancel current op) | global |
| `/` | Filter / search in pane | pane |
| `n` / `N` | Next / previous match | pane |
| `Space` | Toggle multi-select | key list |
| `→` / `l` | Open key in value pane, or expand/descend a tree group | key list |
| `←` / `h` | Collapse a tree group, or move to its parent | key list |
| `Enter` | Open the Selected key (if needed) and start moving a cursor inside it | key list / value pane |
| `r` | Refresh / rescan | pane |
| `e` | Open the inline value editor, or (Hash, cursor on a field) edit that field's value | value pane, focused |
| `a` | Add a field to the open Hash — opens the two-part FIELD/VALUE add form | value pane, focused |
| `Ctrl-S` | Stage the inline editor's buffer for confirmation | value pane, editing |
| `t` | Edit TTL (set / persist / extend) | value pane, focused |
| `c` / `C` | Copy key or value / copy `redis-cli` command | key list, value pane |
| `d` | Stage delete of the Selected key (`DEL`), (Hash, cursor on a field) of that field (`HDEL`), or (Slowlog view) `SLOWLOG RESET` | key list, value pane, slowlog view |
| `y` | Confirm a staged mutation (`Esc` dismisses) | global, only while one is staged |
| `p` | Pause / resume consuming the `MONITOR` or Pub/Sub feed (the socket stays open; paused lines/messages are counted, not buffered) | Monitor view, Pub/Sub tail |
| `a` | Add a subscription | Pub/Sub view, either half |
| `d` / `←→` | Unsubscribe the selected chip / pick a chip | Pub/Sub strip |
| `←→↑↓` / `hjkl` | Move tile focus | Dashboard view |
| `Enter` | Expand the focused tile's raw `INFO` section into a scrollable overlay | Dashboard view |
| `Ctrl-R` | Toggle read-only mode | global |

Bindings are user-overridable in config; the hint bar and the help overlay (`?`) both render the
*effective* binding, so a remap is never a lie the hint bar tells.

**Help is contextual** (R7.5, `crates/core/src/help.rs`). `?` — or `F1`, which also works inside
the filter, the inline editor and the confirm dialog, where `?` is text or is swallowed — opens an
overlay listing what the keys do *here*: a HERE block for the focused context (keys pane; value
pane per type × value cursor; each editor target; filter; confirm), then an EVERYWHERE line of
global keys. Labels name the target (`delete field`, `remove member`, `edit score`), not just the
verb. A key that starts a mutation is dimmed under Read-only Mode with its reason and
`· preview only` — it still stages the preview, which confirm then refuses; `⌃R` is omitted
outright under the `replica` reason. Help is modal: only `Esc`, `?`, `F1` (close) and `Tab` (view
the other pane's keys without moving focus) act while it is open, so pressing `d` to see what it
does stages nothing. Every context fits 80×24 without scrolling. The hint bar is the same row
list — the first rows that fit, with the help key pinned last — so the two cannot disagree.

**Keymap growth rule.** The Palette was the one check against this keymap growing without limit
(CLAUDE.md); withdrawing it (ADR-0020) replaces that check with a written rule instead of a
searchable escape hatch: a bare top-level single key is spent only on a per-session action, one
reached for in most sessions — not a rare or long-tail one, which has no business claiming a whole
key of its own. A view (Dashboard, Monitor, Pub/Sub, Slowlog) never gets a bare key; it lives under
the `g`-prefixed chord above. Everything else is scoped to whichever pane or view already has
focus, so the same key can mean two things in two places without colliding — `r` is refetch in the
value pane and rescan in the keys pane, `t` is tree in the keys pane and edit-ttl in the value pane
— rather than needing a second binding. Revisit this rule, per ADR-0020, once focus-scoped keys and
`g`-chords genuinely run out, or once an overloaded key like `t` starts confusing users.

Derived from the default keymap in `crates/core/src/keymap/mod.rs` (not from the table above,
which includes keys this document plans but this codebase has not built yet): the free single
lowercase keys, unclaimed by any default binding, are `b f i m n o u v w x z`. `g` is free too,
but is reserved as the view-jump chord prefix above rather than as a lone binding. `p` is no longer
free — it is `Action::TogglePause`, scoped to the Monitor view alone (Monitor's own keymap growth
rule: a bare key whose meaning nothing outside that view gives it, exactly like `d`/`e`/`t`/`c`).

**Focus is one concept at every width.** Below 70 columns it decides which pane is *drawn*
(§2's stack navigation); at or above it, both panes are drawn and focus decides only which one a
pane-scoped key acts on. Opening a key moves focus to it, `Esc` moves it back, and `Tab` moves it
without closing the key. Because `r` rescans in one pane and Refetches in the other, focus is
information rather than decoration: the focused pane's header is drawn in full-strength text and
the unfocused one muted (dim, not merely grey, so the distinction survives monochrome), and the
hint bar names the half in force — `r rescan` or `r refetch`. A pane-scoped key whose target the
reader cannot see is a key that does the wrong thing silently, which is exactly what happened
when focus was inferred from "is a key open" instead of tracked.

## 5. Visual language

**Color roles** (semantic tokens, not literal colors — themes remap them):

| Token | Use |
|---|---|
| `surface-detached` | A background wash on the Viewer while it holds a key that is not the Selected key (§6.4). There is no `surface` token: the app never paints the pane background, the terminal's own shows through |
| `border` / `border-focus` | Pane edges; focused pane gets `border-focus` + bold title |
| `text` / `muted` | Primary content vs. metadata (TTL, sizes, counts) |
| `selected` | Selection and cursor row: a full bar, a fixed foreground and background pair |
| `type.string` … `type.json` | One hue per Redis type — consistent everywhere a type appears |
| `env.local` / `env.staging` / `env.prod` / `env.unknown` | Title bar band and confirmation dialogs |
| `danger` / `warn` / `ok` | Destructive actions, expiring TTLs, success toasts |

The Open key's underline is not a colour token's concern: it is a modifier, the same in every
theme, and cannot be themed.

**Environment signaling.** `local` is neutral, `staging` is amber, `prod` is red, and `unknown`
is a distinct fourth treatment — deliberately not a shade of the others, because it means "nobody
told us," not "somewhere between staging and prod." Applied to the title bar band and to
confirmation dialogs. A prod Connection is recognizable from across a desk. `prod` and
`unknown` both start in Read-only Mode ([ADR-0004](adr/0004-untagged-connections-are-read-only.md)).

**Typography and density.** Bold for headers and focused titles, dim for metadata, no
underlining except links. One blank line between logical groups; padding of one column inside
every pane border. Tables get a header row that stays pinned while the body scrolls.

**Iconography.** Nerd Font glyphs when detected, ASCII fallbacks otherwise, chosen so the
layout width does not change between the two. Codepoints with emoji presentation are banned
outright — they render double-width in most terminals and shear the column grid rather than
degrading quietly. An Ad-hoc Connection is therefore marked `~`, not `⚡`.

**Key names in hint bars are words, not glyphs.** `Esc`, `Enter` and `Tab` are spelled out; the
single-glyph forms are the least reliably present characters in a monospace font, and a missing
glyph breaks alignment instead of falling back. Chords keep their compact form (`⌃K`, `d d`).

**Motion.** Used sparingly and only to explain state: progress bar during scan, a 120ms fade on
toasts, a subtle pulse on a value that just changed under a live view. No decorative animation.

**Themes.** Three built-ins: `dark` (the default), `light` and `high-contrast`. Each is a token →
colour palette with a truecolor and a 256-colour column, and a test measures WCAG contrast for
every readable token against the background the palette is designed for (AA, 4.5:1; AAA, 7:1 for
`high-contrast`). Monochrome is not themed: with no hue to vary, every theme degrades to the
same thing. `light` is for light terminals and `dark` for dark ones — the app does not paint the
terminal background, so it cannot make either work on the wrong one (OSC 11 detection is not
attempted).

Chosen with `--theme <name>`, else the config's `"theme"`, else `dark`. Users add their own under
`"themes"`; a theme names the tokens it changes and inherits the rest from a built-in `"base"`
(default `dark`). An unknown token name is a parse error, like any unknown config field.

```json
{
  "theme": "mine",
  "themes": {
    "mine": {
      "base": "light",
      "border-focus": "#0b5cad",
      "selected": { "fg": "#000000", "bg": "#ffd27f" }
    }
  }
}
```

A bare `"#rrggbb"` sets a token's foreground (its background for `selected` and
`surface-detached`); `{ "fg", "bg" }` sets either or both. The 256-colour value is derived from it
by nearest match. A user theme is not contrast-checked: the test covers the built-ins only.

`env.local` is a neutral grey in every theme and distinct from `env.unknown`; green belongs to
`ok` alone. Config files with `theme`/`themes` are refused by binaries that predate them
(unknown fields are a parse error everywhere), so remove them before rolling back.

## 6. Key screens

### 6.1 Launch, and the first-run picker
The normal path has no screen at all. The target resolves deterministically
([ADR-0001](adr/0001-connection-resolution-order.md)) and the app opens straight into the
keyspace browser with the target and Source in the title bar. Bare launch with nothing
configured connects to local Redis; there is no setup step to get to a keyspace.

The picker appears in exactly one case: nothing is configured *and* nothing was detected. It
offers detected local instances (default ports, common socket paths), a "connect to URL" field,
and — because the app never writes the config file
([ADR-0003](adr/0003-app-never-writes-config.md)) — the config path with a ready-to-paste
example Profile. That example is the only teaching moment we get, so it earns real space.

Config parse failures render here too, naming the file with line and column and showing the
offending fragment. A broken config never degrades into a silent fallback to localhost.

### 6.2 Keyspace browser
The centerpiece. Streams `SCAN` results into a virtualized list that stays interactive while
loading, with progress and a cancel affordance in the status bar. Tree mode folds on the
configured separator and shows child counts on collapsed nodes; flat mode is one keypress away.
Metadata columns fill in asynchronously — the key appears immediately, its size arrives when it
arrives, and a pending cell shows a placeholder rather than shifting the layout.

The columns are `KEY`, `TYPE`, `SIZE`, `TTL`. **Element count is not among them** (R2.4). For
strings it prints the same number twice; for collections it is genuinely diagnostic — three
members occupying 1.1 MB means somebody stored blobs as members — but that is capacity forensics,
not the find-a-key-and-read-it loop this pane exists for. It also costs a fourth pipelined
command per key (`HLEN`/`LLEN`/`SCARD`/`ZCARD`/`XLEN`), which competes with `SCAN` for the
connection while the list is still streaming. It stays reachable two ways: the Viewer header
states it on open, and sorting by count surfaces it as a temporary column.

### 6.3 Value viewers
One viewer per type, each with the same frame (header: key, type, size, TTL; body: type-specific;
footer: actions) so navigation muscle memory transfers:

- **String** — syntax-highlighted when JSON/XML/YAML is detected, toggleable raw/pretty/hex.
- **Hash** — two-column table, sortable by field, filterable, inline edit.
- **List** — indexed rows with head/tail jump; push/pop actions surfaced.
- **Set / Sorted set** — member table; zset adds a score column and rank, sortable by either.
- **Stream** — reverse-chronological entry timeline, expandable fields, consumer-group panel.
- **JSON module** — collapsible tree with JSONPath breadcrumb.
- **Binary/unknown** — hex + ASCII dump with offset gutter.

### 6.4 Liveness

**There is no refresh button, because there is nothing to refresh.** The Viewer holds what the
server last said, never a memo keyed by the key's name, so a value cannot go stale behind a
control that claims to update it. The open key is tracked by the server, and when it changes the
server says so and the Viewer refetches once.

What arrives depends on where the user is. At rest, the new value simply lands, with changed
fields briefly highlighted so the change is legible rather than merely present. Scrolled into a
large hash or stream, nothing moves — the header announces it and waits, because pulling a row
out from under a reader's cursor is its own kind of broken. Mid-edit, the update is held
entirely; an unsaved buffer is never touched.

The header carries the state at all times. Ambiguity is the actual defect being designed
against: the failure users learn to distrust is not a wrong value, it is being unable to tell
"the update did nothing" from "nothing changed."

```
┌─ value pane header · liveness readout ───────────────────┐
│                                                          │
│ live and current                                  ● live │
│ an update just landed               ● live · updated now │
│ a read found no change                ● live · unchanged │
│ changed, you are scrolled    ● live · changed 2s ago   r │
│ mid-edit, held back           ✎ editing · changed · held │
│ key deleted on the server               ✕ deleted 3s ago │
│ tracking unavailable         ○ manual · read 14s ago   r │
│ refetch found a change            ○ manual · updated now │
│ refetch found no change             ○ manual · unchanged │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

The four `updated now` / `unchanged` rows are one rule, not four cases: **the header states what
the last read found, and then stops.** They fade after a couple of seconds, because they are an
account of an event rather than a description of the key — after that the resting phrase takes
over. The two `● live` variants were not in the original inventory and are here because the
question they answer does not depend on liveness: `r` pressed by hand on a live key deserves the
same answer as `r` pressed on a manual one. Without them a Refetch that found nothing rendered a
frame identical in every cell to one where the reply was dropped as superseded, or failed, or was
never sent — which is the ADR-0006 ambiguity reproduced by the screen built to remove it.

TTL is a special case worth stating: it counts down locally from the value read at fetch time,
so the most time-sensitive figure on screen is live at no network cost.

A key deleted, expired, or evicted while open keeps its last read value, badged
`✕ deleted 3s ago`, with mutating actions disabled. During an incident the question is almost
always *what was in it*, and that is precisely the moment the answer becomes unrecoverable.

Where the server cannot support tracking — Redis before 6, or no RESP3 — the readout says
`○ manual`, Read age replaces it, and `r` does the work. This is the same rule as the Source
readout in the title bar: the app may choose for you, but it never lets you assume wrongly.
Silent degradation here would recreate the exact frustration this screen exists to remove.

**Whose value is this?** The Viewer holds the **Open key**, and the cursor sits on the
**Selected key**; they are frequently not the same, because opening is explicit and arrowing the
list deliberately does not fire a read and a `CLIENT TRACKING` re-arm per keystroke. Nothing
about that is a freshness problem — the value is live and tracked either way — but left unsaid it
reads as the value pane showing the wrong key, which is the complaint this whole section exists
to answer, one level up.

So the state is stated on both sides of the divider, and the division of labour is: **the Viewer
says what, the divider says where.**

```
│ KEY                    TYPE   TTL ┊ user:8812:session   ⊘ not the selected key │
│ ██user:8812:cart███████zset███12m ┊ hash · 5 fields · 2.1 KB                   │
│ ● user:8812:profile    json    ∞  ┊ ttl 42m                            ● live  │
│ ● user:8812:session    hash   42m ├   ← the Open key's row, underlined         │
│ ● user:8813:session    hash   56m ┊ FIELD        VALUE                         │
```

- The Viewer is **washed** (`surface-alt`), the divider goes **dashed**, and the header carries
  `⊘ not the selected key` — or `⊘ not in the list` when the Open key has no row at all, because
  it is filtered out, folded inside a collapsed group, or waiting to be re-resolved after a
  rescan. The chip is dropped before the key name is: the name is the pane's identity.
- The keys pane **underlines** the Open key's name, and the divider cell on that row becomes `├`.
  Underline is ranked deliberately below the cursor's full-bar highlight — two marks in one list
  only work if one is obviously the junior — and it is the one modifier still free in monochrome
  once the selection has taken reverse video. When the Open key has scrolled out of the window the
  divider carries `▲`/`▼` at its edge instead.
- **When the two agree, none of this is on screen** and the tie glyph is the only trace. That
  coincidence is the point rather than redundancy: it teaches the relationship in the ordinary
  case, so the moment the panes separate reads as a change and not as a puzzle.

The wash is hue and nothing else, so monochrome loses it entirely — the dashed divider, the chip
and the underline are what carry the state there. That is why the wash is never the only signal,
and it is the same rule as everywhere else: losing colour must lose emphasis, never information.

### 6.5 Editing and confirmation
Every mutation is staged, previewed, then confirmed — one chokepoint, whether it deletes a key or
rewrites a value. Committing shows a **command preview**: the literal command that will be sent.
Confirmation friction scales with blast radius — a single-key `y` for one non-prod delete, a
typed key-count for a bulk prod delete. Read-only Mode refuses at the preview, not at the keypress: the dialog composes the real
command and its blast radius first, and only then says you cannot run it. You learn what you
were about to do before you learn that you are not allowed to. `Esc` always discards, at any
stage — consistent with every other overlay in the app, at the cost of losing a draft to a
misplaced keypress, which was a deliberate choice over special-casing edits. **Only `y` confirms
and only `Esc` dismisses the confirm dialog; every other key is ignored rather than discarding** —
a stray or leaked keystroke can never silently throw away a staged mutation (ADR-0014).

**`e`/`a`/`d` act on the Open key only with the value pane focused, and the value fetched.**
With the keys pane focused, moving the cursor there fetches nothing, so `e`/`a` would otherwise act
on whatever key happens to be open rather than the one under the cursor — a short notice (`Tab to
the value pane to edit`, or `open a key first` with nothing open) says so instead of silently acting
on the wrong key. `d` is unaffected: it already targets the Selected key's `DEL` from the keys pane,
a different command on a different target, unchanged by this. With the value pane focused but
nothing ever read for the Open key (a key confirmed gone before it loaded), both refuse with
`nothing open to edit` — or `gone — nothing to edit` if the key was seen and is now gone.

**String values edit inline, in the value pane.** `e` opens an embedded text editor
(`ratatui-textarea`) directly where the value was, replacing the body rows; the header keeps
showing `✎ editing · changed · held` and, for a value that reads as JSON, a live `json ✓`/`json ✗`
indicator next to the TTL. The editor's cursor opens at the start of the line the Viewer's cursor
was on, so `e` edits what you were looking at. After `y`, the value read back is shown at once,
even with the cursor below the top row: this session's own write is never held as an update.
`Ctrl-S` stages the buffer for confirmation — the same command-preview
dialog every other mutation uses, showing a stacked diff (old value in red, new one in green, not
a line-by-line diff) capped to a handful of lines so one long value cannot take over the screen —
and `Esc` discards it outright, with no return to a prior draft. Staging with no actual change
closes the buffer silently rather than opening an empty preview. While the dialog is up, and until
the write is read back, the pane keeps showing the edited text rather than the value it replaces.
The write is `SET key value KEEPTTL XX`: it keeps whatever TTL the key has, and writes only if the
key still exists. A key that is gone by then — expired or deleted under the dialog — is never
recreated: nothing is written, the dialog closes (at once when Liveness sees the key go, otherwise
at `y`), the key is badged gone, and the edited text goes back into the buffer rather than being
lost, where `Esc` still discards it. `Ctrl-Z`/`Ctrl-Y` undo and redo
inside the buffer. A value that already reads as JSON (R3.2) opens pretty-printed; the confirm
dialog still warns, without blocking, if the edited text no longer parses — it is still just a
STRING underneath, and Redis has no opinion on whether its bytes are valid JSON. List, Set, Sorted
set and binary strings are not editable yet; that lands type by type.

**Hash fields edit, add and remove the same way, one field at a time.** With the value cursor on a
field (`Enter` first), `e` opens that field's raw value in the same inline editor, with its name
shown read-only above it; `d` stages removing it. `a` needs no cursor — it opens a two-part form in
place of the body, labelled `FIELD` and `VALUE` like the table's own columns, with a `▌` marker
before whichever half is active (a glyph, not colour alone, so it survives monochrome). `Enter` or
`↓` moves from `FIELD` to `VALUE`; `↑` moves back once the value's cursor has nowhere left to go
(the top screen row, including inside a wrapped first line); `Tab` still inserts a tab in `VALUE`.
While typing the name, a live `⚠ exists` marker appears the moment it matches a field already
fetched, and `Enter`/`↓`/`Ctrl-S` are all blocked until it is corrected — a duplicate outside the
fetched window is still caught only by the guard below, at write time. `Ctrl-S` stages from either
part once the name is non-empty and not a shown duplicate; `Esc` discards the whole add from
either part. All three write through a guarded Lua script rather than
a plain `HSET`/`HSETNX`/`HDEL`, so the confirm dialog shows the effective command it performs —
`HSET user:1 token`, never the literal `EVAL` — with one muted guard line underneath naming what
the script checks first: *only if the field still exists · keeps its TTL* for an edit, *only if the
key still exists · never overwrites a field* for an add. A field that is gone by the time an edit
lands, or already there by the time an add lands, writes nothing; the dialog closes (or, if it
already closed at `y`, a notice says so) and the typed text goes back into the buffer, held under
the same "an open editor is never touched" guarantee as a String edit. Removing a Hash's last field
carries an extra warning in the dialog — *last field — the key will be deleted* — because `HDEL`
deletes the key itself when nothing is left in it. As with a String edit, the key itself is never
recreated if it is gone by the time the write lands. See
[ADR-0015](adr/0015-hash-field-writes-are-guarded.md) for the scripts, the guards, and why a plain
`HSET`/`HSETNX` was rejected.

**Values over 200KB are refused, with a notice.** `e` on a value whose raw byte length exceeds
that threshold says so, and mentions that an external-editor escape hatch is planned for values
too large to hold comfortably in the inline editor — without promising a keybinding, since none
exists yet. The threshold comes from measuring `ratatui-textarea` 0.9.2 in release, keystroke plus
full render, at 120×40 wrapping at words and splitting any word wider than the pane: p99 was clean (≤8.5ms, comfortably inside the 16ms frame
budget) at 100KB and 200KB on every run, while 300KB was noisy across runs — the cost is dominated
by re-wrapping one long logical line, not by the edit operation itself. See
[ADR-0014](adr/0014-values-are-edited-inline.md) for the full numbers and the rejected
alternatives, including why `$EDITOR` as the *default* path was dropped.

**Why not `$EDITOR` by default.** The original build of this feature shelled straight out to
`$VISUAL`/`$EDITOR`/`vi` on a temp file. Manual testing over a real terminal found a live bug: a
child editor process (vim, reliably) queries the terminal's colours at startup over the same tty
this app reads from, and the reply can arrive *after* the editor has already exited and control
has returned — landing on our own input thread and replaying as a burst of keystrokes into
whatever was on screen. That is a known class of bug in other terminal apps, and it is worse over
SSH, where the round trip is slower. The inline editor removes the terminal handoff — and the
race with it — from the default path entirely; the escape hatch keeps `$EDITOR` available,
hardened, for the reader who wants it.

### 6.6 Dashboard

**Dashboard (`g d`, built — R6.3, M3 task 6, `docs/plans/m3-dashboard.md`).** Triage-first: memory
used vs. peak vs. `maxmemory` as a bar (`maxmemory 0` reads "no limit," not a bar with no
ceiling), hit ratio, an ops/sec sparkline over the last 60 polls (~2 minutes), connected/blocked
clients, replication role and lag, and eviction/expiry counters. Anything alarming is colored
through the theme's semantic `Token::Warn`/`Token::Danger` tokens, never a literal colour, and
every tile can be expanded (`Enter`) into the raw `INFO` section behind it as a scrollable
overlay (`Esc` closes it, `Esc` again leaves the view — nearest thing first).

Single node (ADR-0008): `INFO`, like `SLOWLOG`, is per-server, so a Cluster-scoped Dashboard would
need a node selector this app's premise has no room for — out of scope for v1.

Data source is `INFO`, request/response on the main connection, not a feed — unlike Monitor and
Pub/Sub there is no `CLIENT TRACKING` equivalent to arm. `g d` fetches once immediately (no blank
tile waiting for the first tick) and a shell-side `tokio::time::interval` polls it every 2s while
the view is on screen; the timer itself is shell plumbing (the core never owns one, per the
injected-clock discipline), but whether a given tick actually fetches — screen showing, connection
up, no poll already in flight — is a decision the core makes from state alone, so it is testable
without a real clock or a real interval. A manual `g d`/`r` can overlap a timer poll; each fetch
carries a token minted by the core, and a reply whose token is not the current poll's is dropped
whole, so an older, slower reply landing after a newer one cannot overwrite it or corrupt the
counter-based alarms' before/after comparison.

Replication reads differently depending on role, since `INFO` describes the two sides of a
replication link with different fields entirely: a primary's lag is the *worst* lag across its
`slaveN:...,lag=N` lines (any replica whose `state` is not `online` alarms outright, regardless of
lag); a replica's is `master_last_io_seconds_ago` plus `master_link_status` (`down` alarms
outright). A primary with zero replicas is unremarkable, not an error.

Alarm thresholds (named constants in `crates/core/src/state/dashboard.rs`, not user-configurable
for v1): memory ≥80% of `maxmemory` warns, ≥95% is danger; hit ratio below 80% warns, but only once
`keyspace_hits + keyspace_misses` has reached 1,000 samples — a handful of reads on a fresh
container earns no alarm; replication lag above 5s warns, above 30s (or the link down) is danger;
`blocked_clients` above zero warns; `evicted_keys` rising between polls warns; `rejected_connections`
rising between polls is danger, folded into the clients tile rather than given a seventh tile of
its own, since it is a fact about client admission, the same subject that tile already covers.

Grid breakpoints (§2): 3 tiles per row at ≥120 columns, 2 at 80–119, 1 (scrolling) below 80. Tile
focus (`←→↑↓`/`hjkl`) moves through the grid's own row-major order; below 80 columns the grid
scrolls to keep the focused tile on screen, the same stateless-viewport trick the keys pane uses
for its own selection, so no persisted grid-scroll field exists.

### 6.7 Monitor

**Monitor (`g m`, built — R6.1, M3).** A live, unfiltered tail of every command the server
executes, driven by `MONITOR` on its own dedicated connection (`docs/plans/m3-feed-connection.md`)
— never the main one, which stays free for ordinary reads/writes and the Open key's `CLIENT
TRACKING` arming the whole time the tail is open. The feed is opened fresh on every `g m` and
closed on every way out of the view — `Esc`, `g k`, `g s`, quitting — never left running behind a
screen with nothing on it to say so.

```
┌─ g m · Monitor ──────────────────────────────────────────────────────────────┐
│ ⚠ MONITOR is running — the server pays for every command it streams here     │
│ ● live                                                             1,204 lines│
│ TIME         DB  CLIENT                COMMAND                              │
│ 16:21:23.107 0   10.0.0.4:51820        "GET" "user:8812:session"            │
│ 16:21:23.209 0   10.0.0.9:33012        "SET" "lock:checkout:8812" "1"       │
│ 16:21:23.310 0   10.0.0.4:51820        "EXPIRE" "user:8812:session" "1800"  │
└────────────────────────────────────────────────────────────────────────────┘
```

- **The warning banner is persistent, and never truncated** — a fixed row at the top of the view,
  present at every width down to the single-pane floor, dismissible only by leaving the view.
  `MONITOR` costs the server for every connection watching it, server-wide, for as long as it stays
  open; a banner that faded would misstate that as a moment's notice rather than a standing cost,
  and one cut off with an `…` would half-miss it the same way. Two fixed wordings, not one
  truncated to fit: the full sentence above down to the 80-column floor (R7.1), and
  `⚠ MONITOR running — costs the server` below it, at the single-pane width. Once the feed has
  closed the warning would be false, so the same row turns muted and says
  `MONITOR stopped — the server is no longer streaming to this view` instead, keeping the layout
  still.
- **The buffer is bounded and the cap is visible**: 5,000 lines, each truncated to 4 KiB on arrival
  (so one oversized value cannot make a single line bigger than the buffer is meant to be — worst
  case is bytes, not gigabytes). Enforced in exactly one function, the same discipline the Loaded
  set's own cap follows (ADR-0010).
- **Pause (`p`) stops consuming, not just hides.** The socket stays open; a paused line is counted
  (`paused · 340 skipped`) and discarded, never queued — resuming does not backfill what was
  dropped. A naive pause that only stopped rendering would still grow the buffer underneath it,
  trading a visible freeze for an invisible leak. Offered only while the feed is actually `Open` —
  with no feed streaming (connecting, or closed) `p` would be a keypress that did nothing, so it is
  hidden rather than dimmed; the closed/connecting hint bar offers `r reopen` in its place.
- **Following.** The view tracks new lines while the selection sits on the last one; moving up
  stops following (the header says so — `following off · End to resume`) without pausing the feed;
  `End` jumps back to the tail and resumes.
- **Filter (`/`)** reuses the keys pane's own filter capture and glob/substring matching, narrowing
  what's *displayed* only — a filtered-out line still occupies its slot in the bounded buffer, so
  clearing the filter shows exactly what would have been there anyway.
- **Columns**: the server's own timestamp (UTC `HH:MM:SS.mmm`, preferred over receipt time — the
  question a reader has is "when did this run"), then DB, CLIENT, and COMMAND, shed narrowest-first
  in that order as the terminal narrows. `c` copies the selected line's command.
- **A closed feed keeps its buffer on screen.** The header reads `feed closed: <reason>`, an R7.4
  notification names it too, and `r` reopens — no automatic reconnect, the same "secondary view,
  not the always-on Viewer" posture ADR-0006's liveness guarantee is scoped away from.
- **Cost confirmation.** On `prod`/`unknown`, `g m` shows a confirm dialog naming the actual
  Environment (`Open MONITOR on prod?`, not a generic "are you sure") and wrapping the cost
  explanation across as many lines as it needs rather than truncating it — a dialog whose whole
  purpose is naming a cost must not itself lose half a sentence to an `…`. `y` opens, `Esc` cancels.
  This is not a mutation preview — opening a view is not a write — so Read-only Mode never sees it
  and never refuses it.

### 6.8 Pub/Sub

**Pub/Sub (`g p`, built — R6.2, M3 task 5, `docs/plans/m3-pubsub.md`).** Shares the same
dedicated-connection machinery Monitor uses (`docs/plans/m3-feed-connection.md`), but is a
genuinely different shape, not "Monitor with a different source": the reader chooses what to
subscribe to *before* anything streams (Monitor's view opens already running), and every message
carries a channel identity Monitor's raw command text has no analogue of.

```
┌─ g p · Pub/Sub ────────────────────────────────────────────────────────────┐
│ ⟡ [orders] [user:* ⁎]                                              a add    │
│ ● live                                                          3 messages │
│ TIME         CHANNEL               PAYLOAD                                 │
│ 16:21:23.500 orders                {"id":8812,"total":41.5}                │
│ 16:21:23.501 user:42               login                                   │
│ 16:21:23.502 orders                {"id":8813}                             │
│ ──────────────────────────────────────────────────────────────────────────│
│ channel orders                                                             │
│ {                                                                          │
│   "id": 8813                                                               │
│ }                                                                          │
│ Tab focus strip/tail   a add   p pause   / filter   c copy payload   ? F1 help │
└──────────────────────────────────────────────────────────────────────────┘
```

- **A one-row subscription-chip strip**, not a Monitor-style banner: `⟡`/`▶` marks whether the
  strip or the tail currently has focus (`Tab` flips it), each subscription is a `[name]` chip —
  a pattern's carries a trailing `⁎` — and `a add` is always shown at the right and works from
  either half. The view opens with the **tail** focused, since a reader comes back to read
  messages; the strip is where `Tab` takes you to pick a chip (`←→`) and unsubscribe it (`d`).
  This is the "configure before you see anything" step Monitor has no equivalent of.
- **Channel or pattern is auto-detected**: `*`, `?` or `[` anywhere in the typed text makes it a
  pattern (`PSUBSCRIBE`); anything else is a channel (`SUBSCRIBE`); a leading `=` forces a channel.
- **No persistent warning banner and no `prod`/`unknown` confirmation** — unlike `MONITOR`'s
  server-wide, involuntary cost, a Pub/Sub subscription costs only what the reader chose to
  subscribe to. Subscribing is not a write, so Read-only Mode never sees it either.
- **The subscription list is remembered for the session, never persisted.** Leaving the view (any
  route: `Esc`, `g k`/`g s`/`g m`, quitting) closes the feed connection — nothing stays subscribed
  server-side — but the list itself stays in `State`, and `g p` resubscribes to it. `d` on a chip
  unsubscribes it immediately, locally and (while the feed is live) on the server, via the same
  connection — never a close-then-reopen, which would risk dropping a message on every other
  channel already subscribed.
- **The connection opens lazily.** `g p` with no remembered subscription opens with the add-input
  focused and dials nothing until the first one is added.
- **Bounded like Monitor**: 5,000 messages, each payload truncated to 4 KiB on arrival, enforced in
  exactly one function. `p` pause counts rather than buffers; `/` filters channel and payload for
  display only; `End` resumes following; `c` copies the selected message's payload.
- **Columns**: the shell's own receipt time (Pub/Sub messages carry no server-side timestamp,
  unlike `MONITOR`'s lines), then CHANNEL, then PAYLOAD — TIME sheds first below 80 columns,
  CHANNEL is folded into the payload text (never dropped outright) below 70. This is the direct
  answer to "distinct from Monitor's layout": the tail is at minimum two columns where Monitor's
  is one.
- **A detail strip** for the selected message — channel, `via <pattern>` when a pattern matched,
  and the full payload, pretty-printed when it parses as JSON, with the Viewer's own byte
  escaping throughout.
- **A known limit on `via` when patterns overlap.** Redis delivers one `pmessage` per matching
  pattern, so a channel matching two subscribed patterns at once (e.g. `user:*` and `user:4?`)
  arrives as two separate messages with identical channel and payload — and fred's public API
  (the Redis client this app is built on) does not expose which pattern produced which delivery.
  When more than one subscribed pattern could match, the detail strip hedges rather than asserts:
  `via user:* (or another matching pattern)`. Reading raw RESP frames ourselves to resolve this
  exactly was considered and rejected — see `crates/app/src/redis/feed.rs`'s `matched_pattern`.
- **A failed subscribe/unsubscribe surfaces as an R7.4 notification** naming the failing command
  (`SUBSCRIBE`/`PSUBSCRIBE`/`UNSUBSCRIBE`/`PUNSUBSCRIBE`) — an ACL's `-NOPERM` on a restricted
  channel, most plausibly. A chip whose `SUBSCRIBE` failed is dropped rather than left showing as
  subscribed when the server never actually subscribed it.

### 6.9 Connection states and degradation

The title bar already answers *what am I connected to, and why*. It also has to answer *is that
still true*, and *can I write*. Both are chrome the user reads without looking for it, so both
live in the same place.

```
┌─ title bar · connection and safety readout ──────────────────┐
│                                                              │
│ healthy, writes allowed             ● staging · from profile │
│ read-only by Environment          READ-ONLY environment   ⌃R │
│ read-only: target is a replica    READ-ONLY replica   locked │
│ read-only by choice                      READ-ONLY user   ⌃R │
│ maxmemory reached                    ✕ OOM · writes rejected │
│ RDB save failing                 ✕ MISCONF · writes rejected │
│ server restarting                              ⟳ loading 43% │
│ connection lost                ✕ disconnected · retry 4s   r │
│ reconnected                              ● tracking re-armed │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

**Losing the Connection is not an error screen.** Reconnection runs in the background with a
visible backoff countdown, the app stays interactive, and the Viewer keeps its last read value
badged rather than clearing — the same promise as a deleted key (§6.4). `r` retries immediately
instead of waiting out the timer, because a silent wait is a freeze wearing a different name.

**A reconnect re-arms tracking before it claims to be live.** Tracking is per-connection state,
so a transparent reconnect leaves the server no longer watching the open key. The header must
not read `● live` until it does. This is the one invariant in the design that, if it rots, puts
the product back where RedisInsight was.

**Read-only Mode shows its reason.** It can be on because the Environment is `prod` or
`unknown`, because the server reported `role:slave`, or because the user asked. Only the first
and third are liftable; a replica will refuse writes whatever the app believes. Offering `⌃R`
where it cannot work would be a toggle that lies, so the hint reads `locked` instead.

**Failing at launch is not this screen.** A target that cannot be reached exits to the shell with
a diagnostic naming the target, its Source, and the failure
([ADR-0009](adr/0009-connection-lifecycle.md)). Mid-session there is data worth keeping on
screen; at startup there is nothing to show, and an app that opens to an empty error box wastes
the reader's time.

### 6.10 Slowlog

Reached by `g s` (§3), a full-screen `View` — not a third pane (G7) — showing the server's own
`SLOWLOG GET` ring buffer in the same header/list/detail-strip frame every screen in this app
uses, so the muscle memory built browsing keys transfers here too (PLAN's "Proves: entries render
in a type-aware-consistent frame"). `Esc` or `g k` returns to the Keys view exactly as it was
left — nothing about the browser (the Open key, its scroll position, its filter) is touched by
visiting Slowlog, and tracking on the Open key survives the round trip untouched.

The list is four columns — AGE, DURATION, COMMAND, CLIENT — CLIENT sheds first below 80 columns
and AGE second below 70, the same "shed the least essential first, never the reason the screen
exists" rule §2's breakpoints apply to the keyspace browser's own columns. AGE is relative to the
clock (`12s ago`, `3h ago`, `2d ago` — a bare `HH:MM:SS` would be ambiguous for an entry days
old); DURATION above 100ms is colored as a warning, a threshold this screen draws for itself,
distinct from the server's own `slowlog-log-slower-than` (which is what put the entry in the log
at all). A detail strip under the list shows the selected entry's full command (the Viewer's own
byte escaping — a binary argument reads the same way a binary collection member does), client
address/name, and the exact moment as UTC with date, since the AGE column is deliberately coarse
and relative instead.

`s` cycles sort (recent → slowest), `r` refetches the whole buffer — there is no partial refresh,
matching "there is no refresh button" everywhere else in this app (§6.4) — `c` copies the selected
command, and `d` stages `SLOWLOG RESET`: a real mutation through the one chokepoint every other
write goes through (§6.5), previewed as a single line with no guard (it destroys only diagnostic
history) and confirmed with one `y`, refused at confirm under any Read-only Mode reason including
`replica` — this app's read-only rule applies uniformly, with no carve-out for a write that
touches no key. A successful reset refetches the view, the same "the reply is what reaches the
screen, never what this session already knew it sent" discipline every other confirmed write
follows.

A failed fetch is never a blank screen: an R7.4 notification names the failing command, and the
list itself explains why it has nothing to show — still fetching, the fetch failed, or the ring
buffer is genuinely empty (a real state right after a fresh reset, or simply nothing having
crossed the server's threshold yet) — three different reasons, three different sentences, not one
placeholder standing in for all of them.

Single-node only ([ADR-0008](adr/0008-sentinel-in-v1-cluster-deferred.md),
[ADR-0021](adr/0021-cluster-refused-until-supported.md)): `SLOWLOG` is per-node, and a Cluster
target would need a node selector this app's one-Connection premise has no room for — inherited
scope from Cluster being out of v1 entirely, not a new limitation this screen introduces.

## 7. Interaction details that carry the product

- **Optimistic focus.** Opening a key renders header and metadata instantly from what the list
  already knows, then fills the body when the fetch lands. No blank frame. What the list "knows"
  is a hint for the first frame only — it is never the value (§6.4).
- **Cancellation everywhere.** `Esc` aborts an in-flight scan, fetch, or command and says so.
- **Toasts, not dialogs, for outcomes.** Errors include the failing command and a copy action.
- **Persistent session state.** Pane split, filter, and scroll position restore on relaunch,
  keyed by target — reopening `redis-pane staging` feels like never having left, and it does not
  drag staging's filter into a prod session. This lives in a state file under
  `$XDG_STATE_HOME/redis-pane/`, never in the user's config file.
- **Copy that fits the terminal.** `y` offers key / value / `redis-cli` command / permalink-style
  reference — because the next step is usually pasting into a ticket or a shell.
- **Mouse support (R7.3), never required.** Click focuses a pane; the wheel scrolls whichever
  pane is under it, focusing that pane the same way a click would, so a keyboard action right
  after does not silently land on the other one; dragging the divider itself resizes the split.
  Every one of those is also reachable from the keyboard (`Tab`, `↑↓`/`⌃↑↓`, `⌃←→`) — this is a
  keyboard-first tool used over SSH as often as at a desk, so the mouse is a shortcut for the
  motions above, never a second way to reach something the keyboard cannot.

## 8. Accessibility

- Never encode meaning in color alone — pair every color signal with a glyph, label, or weight.
- WCAG AA contrast for every built-in theme, enforced by a test; `high-contrast` is the third, at AAA.
- Full monochrome fallback that remains navigable. The type name stays in the key list when
  color is gone, so a hash is still distinguishable from a sorted set.
- Screen-reader-friendly mode: linearized rendering, no box-drawing, announced focus changes.
- No timing-dependent interactions; every chord is listed in the help overlay (`?`) with its
  binding, so none has to be recalled from memory.

## 9. Open design questions

- Does the keys pane need a permanent column header row, or can the columns be implied by the
  data and explained once in help?

**Resolved since M3 task 6** — the Dashboard belongs in v1: re-decided at `docs/plans/m3-dashboard.md`'s
own decision point once the Slowlog had shipped and the gap it did not cover (memory against
`maxmemory`, replication lag, a client spike) was judged real rather than assumed. Built as §6.6
describes.

**Resolved since v0.6** — the split defaults to each density's documented ratio (45% keys at
Full, 50% at Tight/NoSize) and is independent of focus, not a function of it: focus already
answers a different question — which pane a pane-scoped key acts on (§4) — and overloading it to
also mean "which pane is bigger" would make moving focus resize the screen out from under the
reader. `⌃←`/`⌃→` nudge the divider from that default in either direction, clamped so neither
pane can be squeezed below a usable width; the offset is one number, held for the session and
applied identically whichever density the terminal is currently at, so widening the terminal past
a breakpoint reshuffles columns the documented way without discarding a reader's adjustment.
Mouse drag-to-resize (R7.3) is unbuilt and will drive the same offset. Not yet persisted across a
relaunch — §7's "pane split, filter, and scroll position restore on relaunch" needs the
session-state file ADR-0003 describes, which does not exist yet either.

**Resolved since v0.5** — the keys pane does not get liveness, and the open key remains the only
tracked thing. Deliberate, not deferred: RedisInsight declines to auto-refresh its key list for
the same reason, and re-walking a million keys on a timer is what `SCAN`-not-`KEYS` exists to
avoid. Viewport-scoped `CLIENT TRACKING` — arming only the ~30 visible rows — is the middle
ground ADR-0006 never considered, and it is rejected here on tracking-table churn during scroll;
revisiting it needs its own ADR. What the pane gets instead is the half that was already on the
wire and being discarded: `fetch_metadata` issues `TYPE` for every visible row and sees `"none"`
for a key that has been deleted, expired or evicted, so those rows are now badged `✕ … gone` at
no extra round trip. A gone row is re-checked whenever it is back in the visible window — it rides the same pipelined `TYPE` batch, so there is no extra round trip, and a key deleted and then recreated loses its badge as soon as it is scrolled to — and it **keeps its position** — removing it would renumber everything
below the reader's cursor between one frame and the next — and keeps its last-known size, which
is usually the only answer left about a key during an incident; its TTL becomes `—`, because a
countdown is a claim about a key that is no longer there to expire. Anything beyond deletion
needs the keyspace walked again, which is what `r` in the keys pane now does (R2.7 — documented
from the start, and unimplemented until now). The value header states its own window the same
way: `12,000 items · 500 shown`, since `LLEN`/`ZCARD`/`XLEN` and the 500-row read window are
different numbers and printing only the first turns a slice into the whole.

**Resolved since v0.4** — the scan cap gets a persistent banner row above the key list, not just
a status-bar line: a copy confirmation or a sort readout could otherwise displace the one signal
that what is on screen is a prefix of the keyspace, not the whole of it — a wrong "no matches"
looking identical to "never scanned that far" was the actual risk. Reserved only while capped
(G7), and correctly survives filtering, sorting and tree/flat toggling, none of which re-scan.
Stack navigation below 70 columns is built: `Open` pushes from the key
list to a full-width value pane with a breadcrumb header (`Esc back · key-name`) in place of the
column headers there is no room for; `Esc` pops back, ahead of an unrelated in-flight scan but
behind closing help or dismissing an error. Tree is the default key view (R2.3): fewer rows at rest outweighs the
one extra keypress to reach a leaf, decided from real use against seeded keyspaces on Upstash and
Redis Cloud rather than from the mockup alone.

**Resolved since v0.2** — the sidebar (removed; [ADR-0005](adr/0005-one-connection-per-process.md)),
tabs vs. sidebar for multiple Connections (dissolved with it), the Console's shape (an overlay;
a persistent split is exactly the resident chrome G7 forbids), and the element-count column
(dropped; see §6.2).
