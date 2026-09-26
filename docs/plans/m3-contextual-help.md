# Contextual help (`?` / `F1`)

Status: **done.** Follows [`m3-palette-withdrawn.md`](m3-palette-withdrawn.md)
(ADR-0020), which moved the Palette's one surviving job — discovery for the occasional on-call
user — here. Implements the reworded R7.5.

## Context

Today `?` toggles a flat list of every binding (`render::help_lines`): 35 rows, which does not
fit 80×24, with aliases duplicated (`↓` and `j` on separate rows) and labels that cannot say what
a key acts on (`delete`, not `delete field`). Meanwhile the hint bar (`render::hint_bar`) is
already contextual — a hand-written ladder over mode, focus, open type and value cursor — but has
room for about five entries, and help and hint bar are two unrelated code paths.

The fix is one model of "what the keys do here", read by both: help shows all of it, the hint
bar shows the first rows that fit. A single context is 8–12 rows, so it fits 80×24 with no
scroll and no filter.

## Decisions (settled 2026-09-26)

1. **Scope:** HERE (the focused context) + EVERYWHERE (global keys) + one pointer line to the
   other pane. No filter, no scroll.
2. **Fine-grained contexts** — the states the core already distinguishes:
   - keys pane (flat / tree; with an applied filter or not)
   - filter capture
   - value pane, per open type — String, Hash, List, Set, ZSet, Stream, JSON, Binary — × value
     cursor on/off; value pane with nothing open
   - inline editor, per `EditTarget` (whole value, Hash field, add form name / value part, List
     add, ZSet score, TTL)
   - staged confirmation
3. **Refused actions are dimmed with the reason**, never hidden: `read-only (environment)`,
   `disconnected`, etc. `⌃R` is **omitted** (not dimmed) under the `replica` reason — never offer a
   toggle the server will refuse (CLAUDE.md). Note Read-only Mode is decided at confirm, not at
   staging (`update::mode`'s comment): a mutation key still stages a preview in Read-only Mode, so
   its reason reads e.g. `read-only (environment) · preview only`. **Do not invent policy** —
   every refusal shown must be what dispatch would actually do; derive it from the existing
   gates (`pane_is_on_screen`, `open_editor`'s ladder, confirm's read-only check, liveness).
4. **One source.** Help and hint bar both read the same row list. Hint bar = optional status
   prefix (see below) + HERE rows, then EVERYWHERE rows, truncated to the width, with the help
   binding (`? help`) always pinned last so help stays discoverable. This **changes the default
   Normal-mode hint bar** from globals-only (`Esc back r rescan ⌃R read-only ? help q quit`) to
   context verbs first — intended; DESIGN's own mock already shows pane verbs in the bar.
5. **`F1` is a global alias for `Action::Help`**, and works from every mode — including filter
   capture, the editor, and the confirm dialog, where `?` is a typed character or is swallowed.
   `?` keeps working wherever it does today (Normal mode).
6. **Labels only**, naming the target: `delete field`, `remove member`, `edit score`,
   `stage`, `head/tail`. No description column.
7. **`Tab` inside help** switches the help *view* to the other pane's context; focus does not
   move. Only offered for the two pane contexts (not in editor/filter/confirm contexts, where
   the pointer line is absent).
8. **Help is modal.** While open, only `Esc`, `?`, `F1` (close) and `Tab` (switch view) act;
   every other key is ignored — pressing `d` to "see what happens" must not stage a delete.
   Today help is non-modal (keys pass through and `Cancel` closes it); that changes.
9. **R7.5 reworded** (in this change): "Contextual help overlay (`?`/`F1`) listing every binding
   in force in the focused context plus global ones, dimmed with a reason where refused; the hint
   bar is its first N rows, always visible."

## Architecture

### Core — new `crates/core/src/help.rs`

- `pub enum HelpContext { … }` — derived by `pub fn context(state: &State) -> HelpContext`, in
  the same precedence `update::mode` uses (confirm → editor → filter → Normal, then focus / open
  type / cursor). Reuse `mode()` rather than re-deriving precedence.
- `pub struct HelpRow { keys: String, label: Cow<'static, str>, refused: Option<Refusal> }` —
  `keys` is the **effective** binding(s) resolved through `state.keymap` (so user overrides show),
  aliases merged into one row (`↑↓ jk`). Some editor keys are not rebindable `Action`s (`Enter`
  stage, `Tab` head/tail — see the comments in `hint_bar`); those rows carry the literal key, as
  `hint_bar` already spells them out.
- `pub fn here(state, ctx) -> Vec<HelpRow>` in rank order; `pub fn everywhere(state) ->
  Vec<HelpRow>`; `pub fn status(state) -> Option<String>` for the bar's non-row messages that
  exist today (`field exists — Esc, then e to edit`, `member exists — …`, `invalid score`).
- Build on `Action::label_in` and `Action::pane_is_on_screen`; do not duplicate them. Where
  `hint_bar` encodes a domain rule in a comment (Set has no `e`, ZSet `e` edits the score, `d` in
  the keys pane is `DEL` even with a Hash cursor active, List add's `Tab`), carry the rule *and
  its comment* into `help.rs` — those comments are the spec.
- Rank order per context: most-used verbs first, motions last. Suggested keys pane:
  `→ open`, `/ filter`, `s sort`, `t tree`, `d delete`, `c copy`, `C copy redis-cli`, `r rescan`,
  `↑↓ jk move`, `PgUp/PgDn page`, `Home/End top/bottom`. Value pane (Hash, cursor on):
  `e edit field`, `a add field`, `d delete field`, `c copy`, `t edit ttl`, `r refetch`, motions.
  EVERYWHERE: `Tab focus`, `⌃R read-only`, `Esc back`, `q quit`, `? help`.

### State / update

- `State::help_open: bool` → `State::help: Option<HelpView>`, where `HelpView { pane: Pane }` is
  the context being *viewed* (starts at the focused pane; `Tab` flips it).
- `update::mode` gains `Mode::Help`, ranked **first** (above Confirm): help drawn over a confirm
  dialog is on top, so it owns the keys. Route to a new `help_key` handler: `Esc`/`?`/`F1` close,
  `Tab` flips `HelpView::pane` (pane contexts only), everything else is a no-op.
- `F1` opens help from every mode: handle it before the mode routing in `key_press`, via the
  keymap (`Action::Help` bound to both `?` and `F1`), not a hard-coded key check.
- Remove the `help_open` branch in `cancel` (help's own handler closes it now).

### Keys

- `msg::KeyCode` gains `F(u8)`; `crates/app/src/terminal.rs` `translate` maps function keys. The
  test `an_untranslatable_key_is_dropped_rather_than_guessed_at` uses `F(7)` — switch it to
  another key crossterm reports that the core has no use for (e.g. a media key or `CapsLock`).
- Default keymap: add `F1 → Action::Help`. `key_label` renders it `F1`.

### Render

- Replace `help_lines` + `help_overlay` with a contextual overlay: title
  `help · <context>` (e.g. `help · value pane · hash`), a HERE block, an EVERYWHERE block (may
  pack several short rows per line to save height), refused rows drawn with a dim token plus
  the reason in a warn token, and a footer: `Tab <other pane> keys · Esc close` (pointer only in
  pane contexts). Semantic tokens only; if no dim token exists, add one to the theme rather than
  a literal.
- `hint_bar(state)` becomes: `status(state)` prefix if any, then `here` + `everywhere` rows as
  `key label`, fitted to the bar width, `? help` pinned last. The hand-written ladder goes away.
- **Must fit 80×24**: the overlay for every context fits in the rows available at 24 lines.

## Tests

- **Hint bar tests are the behavioural spec.** The existing `hint_bar_tests` /
  `zset_hint_bar_tests` modules in `render/mod.rs` encode rules that must survive (Tab hidden
  when there is nothing to focus, `d` is `remove` only with the value pane focused, invalid-score
  indicator, …). Keep every assertion about *meaning*; update only assertions that pinned the
  exact old string layout, and list each changed assertion in the PR description.
- Unit (`help.rs`): for **every** context, the hint bar's rows are a prefix of help's rows (after
  the status prefix); no context produces more rows than fit at 80×24; `replica` omits `⌃R`;
  read-only dims mutation rows with `preview only`; disconnected shows `r reconnect`; a user
  rebinding shows in both help and hint bar.
- Update: help is modal (`d` while open stages nothing), `Tab` flips the view without moving
  focus, `F1` opens help from filter capture, the editor, and confirm; `?` in the editor still
  types `?`.
- Golden frames (`tests/golden.rs`, regenerate with `UPDATE_GOLDEN=1`): replace
  `help_overlay.txt` with per-context frames at 80 and 130 columns — keys flat, keys tree, value
  Hash cursor on, Set cursor on, ZSet cursor on, String, editor (Hash field), confirm, read-only
  dimmed, disconnected. Review every regenerated golden diff, including the existing ones whose
  hint bar line changes.

## Docs (same change)

- PRD R7.5 reworded (decision 9).
- DESIGN §4: `?` row → `?` / `F1`, contextual; add a short "Help" paragraph describing HERE /
  EVERYWHERE / dimming / modality. Update the mock (§2) hint bar if it no longer matches.
- `docs/PLAN.md` §6: note contextual help under M3 task 1's withdrawn row.
- CONTEXT.md: add **Help** if the glossary needs the term (HERE / EVERYWHERE are UI copy, not
  glossary terms).

## Verification

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo tree -p redis-pane-core -e normal | grep -iE 'crossterm|tokio|fred'   # must print nothing
```

By hand (`./scripts/redis-up.sh`, `python3 scripts/fixtures.py --flush`, `cargo run -p redis-pane`):
`?` on the keys pane and on each value type; `Tab` inside help; `F1` inside the editor and the
filter; dimmed rows under a `prod` Profile and while disconnected (`./scripts/redis-up.sh down`);
resize to 80×24.
