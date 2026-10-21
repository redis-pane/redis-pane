# Pre-RC UX: `←` leaves the Viewer, `o` returns to the Open key

## Context

Two navigation papercuts before the RC:

1. **`←` in the Viewer.** `Esc` is today's only way from the Viewer back to the key list (`Tab` also works but keeps
   the value cursor). `←`/`h` is bound to `Action::CollapseGroup`, and in the Viewer it is either a no-op (value cursor
   on, or below 70 cols) or, worse, a latent bug: with the cursor off at ≥70 cols, `collapse_group`
   (`crates/core/src/update/keys.rs:331`) has no focus check, so it collapses the tree group under the *key-list*
   cursor, behind the user's back. Since `→`/`l` already opens a key and focuses the Viewer, `←`/`h` leaving it makes
   the two a symmetric pane navigation.
2. **Getting back to the Open key.** After browsing away from the Open key in the key list (the Viewer shows
   `⊘ not the selected key`, i.e. `Detached`/`DetachedOffList`, ADR-0012), there is no way to bring the selection back
   to it short of scrolling.

Decisions taken with the user:
- `←`/`h` in the Viewer, in Normal mode (any text-entry mode already captures keys first), does **exactly `Esc`'s
  focus step**, value cursor on or off: focus → Keys, `cursor_active = false`. Nothing else Esc does (clear error,
  leave full-screen view, cancel bulk delete, clear marks, cancel scan) is borrowed. Cost accepted: `←` is spent in
  the Viewer, so any future horizontal scroll needs another key.
- New key **`o`** = "go to Open key", **key-list scoped only** (hidden from hints/help and a no-op when the Viewer is
  focused). It moves the selection and scroll onto the Open key's row, focus unchanged.
- If the Open key has no row: **unfold its collapsed ancestors and clear the filter**, then land on it.

## Feature A — `←`/`h` leaves the Viewer

- Extract the focus step at `crates/core/src/update/mod.rs:937-943` (inside `cancel()`) into a helper, e.g.
  `fn focus_keys_pane(state: &mut State)`, and have `cancel()` call it. One implementation, two keys.
- In `collapse_group` (`update/keys.rs:331`), first: `if state.focus == Pane::Value { focus_keys_pane(&mut state);
  return (state, Vec::new()); }` — this replaces the `cursor_active` guard's no-op and the latent key-list collapse.
  Leave `open_selected` (`→`) unchanged, except keep its existing cursor-on no-op.
- `Action::pane_is_on_screen` (`crates/core/src/keymap/mod.rs:165-243`): `CollapseGroup` currently gates on the Keys
  pane being drawn, which makes `←` dead in the single-pane Viewer (<70 cols). Make it true when `focus == Value`
  too, so `←` leaves the Viewer at every width (matching Esc).
- Help/hint labels: `label_in` / `help::value_rows` (`crates/core/src/help.rs:847`) gain a `← h  back to keys` row
  in the Viewer context (both cursor on and off). Keep `keys_rows`' `collapse / parent` as is.
- Fix the existing test `left_and_right_do_not_move_the_key_list_while_the_cursor_is_active`
  (`update/viewer.rs:813`): `←` now leaves the pane (still must not move the key list); `→` stays a no-op.

## Feature B — `o` returns the selection to the Open key

- Add `Action::GoToOpenKey` (`keymap/mod.rs:14-155`), default binding `o` (`impl Default for Keymap`, :374-579), add
  it to `every_action_has_a_default_binding`'s list (:816), `label()`/`label_in()` → "open key" (pick wording that
  reads with the glossary: it is the *Open key*, not the Selected key — check CONTEXT.md).
- `pane_is_on_screen`: true only when the Keys pane is focused (`focus == Pane::Keys`) on `View::Keys`; full-screen
  views never see it. Add the dispatch arm in the two-pane browser match (`update/mod.rs:705-846`).
- New `fn go_to_open_key(state)` in `update/keys.rs`:
  1. No `state.open` → `Command::Notify { text: "no key is open" }` (never a silent no-op, CLAUDE.md).
  2. `open.index == None` (rescan not yet re-resolved, or past the Loaded cap) → Notify "the open key isn't in the
     scanned keys yet".
  3. `open.row == Some(row)` → `state.view.selected = row; after_move(state)` (same pattern as `Top`/`Bottom`,
     `update/mod.rs:807-814`). Already on it → no-op, no notify.
  4. `open.row == None` (filtered out and/or folded away):
     - If the filter excludes it: clear the filter (the same state reset the filter's own clear path uses — find it in
       `filter_key_keys_pane`'s Esc/empty handling, `update/keys.rs:~175`, and reuse it rather than writing a second
       one) and Notify "filter cleared to show the open key" so the change is never silent.
     - If `Tree` has a collapsed ancestor of `open.name`: new `Tree::expand_ancestors(&mut self, name: &[u8]) -> bool`
       (`crates/core/src/state/tree.rs`, next to the private `ancestor_collapsed` at :481, handling both the regular
       separator path and the `irregular` path) that removes every collapsed prefix of `name`.
     - Then one rebuild through the existing async paths (`rebuild_list_async` / `refold_async`,
       `state/rebuild.rs:548/570`; whichever covers both list and tree — a filter change needs the list rebuild,
       which also refolds), set `state.follow = Some(index)`, call `settle_follow` (`update/rename.rs:520`) for the
       synchronous case. The sliced case lands through the existing `rebuild_step` → `follow_after_swap`
       (`update/keys.rs:169`). Follow the rename path at `update/rename.rs:410-418` as the model.
  - The deleted (`✕ gone`) Open key still has a row; it lands like any other.
- Hint/help: a `keys_rows` row `o  go to open key` (`help.rs:710`). It is fine for it to appear only in the
  key-list context; the hint bar is a width-truncated prefix of help, so place it after the core rows.

## Docs (same change, CLAUDE.md rule)

- `docs/DESIGN.md` §3 (~113-157, Left/Right semantics and the Esc pane stack) and the §4 keymap table: `←/h` in the
  value pane = back to keys; new `o` row (key list scope); remove `o` from the §4 free-letter list (:227-232).
- `docs/PRD.md`: if a navigation requirement covers this, cite/extend it; otherwise no change.
- User guide: `site/content/docs/browsing/key-list.mdx` ("Moving" table :22-29, Selected vs Open :33-37 — describe
  `o`, incl. that it clears the filter / unfolds groups), `site/content/docs/viewing/types.mdx:12-14` (`←` like Esc),
  `site/content/docs/viewing/live-updates.mdx:64-67` ("When the open key isn't the selected key" → press `o`).
- Regenerate `site/content/docs/reference/keybindings.md`:
  `UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference`.
- `CHANGELOG.md` Unreleased: both features, plus the fixed `←` collapsing the key-list group from the Viewer.
- Persist the plan to `docs/plans/pre-rc-left-and-go-to-open.md` before implementation (user's workflow).

## Tests (TDD, in `crates/core`)

- `update/viewer.rs` / `stack_navigation_tests` (`update/mod.rs:1739`):
  - `←` with Viewer focused, cursor off → focus Keys, key list selection/tree unchanged (regression for the latent
    collapse; use tree mode with the selected row inside an expanded group).
  - `←` with cursor on → focus Keys, `cursor_active == false`, open key unchanged.
  - `h` same as `←`; `←` in single-pane (<70 cols) Viewer → focus Keys.
  - `←` in `Mode::Editing` still moves the text cursor (`update/editor.rs:755`) — unchanged.
- `update/keys.rs` new `mod go_to_open_tests`: no open key → Notify; open row visible-but-scrolled-away → selected =
  row and `view.offset` brings it into view; filtered out → filter cleared + Notify + lands; folded in a collapsed
  group (and nested groups) → ancestors expanded, lands; both filtered and folded; large keyspace (≥ `REBUILD_SLICE`)
  → lands only after the job's final `Msg::RebuildStep`; `open.index == None` → Notify; `o` from the Viewer → no-op
  and hidden.
- `state/tree.rs`: `expand_ancestors` regular and irregular separator cases.
- Keymap: `the_defaults_resolve` still passes (`z` sentinel untouched); help/hint tests in `help.rs` and
  `render/mod.rs` `hint_bar_tests` (`assert_hint_bar_is_a_prefix_of_help`).
- Golden frames: re-record only what changes (help overlay, hint bars) with
  `UPDATE_GOLDEN=1 cargo test -p redis-pane-core`; review each diff by eye.

## Execution

Delegate to one Sonnet subagent in a worktree, on a branch off `main` (e.g. `ux/left-leaves-viewer-o-goes-to-open`),
two commits (A, then B + docs). Checkpoints where I review: after Feature A is green; after `go_to_open_key` +
`expand_ancestors` with tests; after docs/regeneration. Then open a PR; merge only when all checks are green.

## Verification

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  both `docs_reference` tests without `UPDATE_DOCS` (fresh), `pnpm --dir site build` for the guide.
- Boundary check prints nothing: `cargo tree -p redis-pane-core -e normal | grep -iE 'crossterm|tokio|fred'`.
- By hand: `./scripts/redis-up.sh && python3 scripts/fixtures.py --flush && cargo run -p redis-pane` — open a key,
  `←` back (cursor on and off, tree mode, narrow terminal); move away, `o` returns; filter so the open key disappears,
  `o` clears it; collapse its group, `o` unfolds; `fixtures.py -n 200000` for the sliced-rebuild landing.
- No change touches the connection/read path, so the integration suite is not required (CI runs it anyway).
