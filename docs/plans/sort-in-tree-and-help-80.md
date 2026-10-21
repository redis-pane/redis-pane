# `s` in tree view (#108), the help overlay's global line at 80 columns (#111)

Status: **planned** 2026-10-21. One task, one Sonnet subagent, one PR. It merges once every
check has finished and passed.

## Context

Two small UI defects, both open on GitHub.

- **#108.** In tree view `s` does nothing. `cycle_sort` (`crates/core/src/update/keys.rs`)
  returns early when `state.target_tree_mode()`, because folding needs name order. The hint bar
  and the `?` help still offer `s sort`, and nothing on screen says why the key did nothing. That
  breaks the CLAUDE.md rule that an action never vanishes silently.
- **#111.** At 80 columns, the `?` help overlay's packed line of global keys (`help::everywhere`,
  drawn in `render/mod.rs` around `everywhere_line`) is clipped, so `quit` and `help` are cut off.
  80×24 is the minimum supported size (PRD R7.1), and the golden frames `help_*_80` show the clip.

## Deliverables

### 1. `s` in tree view (#108)

- **Pressing `s` in tree view says so.** It shows a non-blocking notice (`state.notice`, the
  same mechanism as `copied …`), for example `tree view is sorted by name · t for flat view`.
  Spell the `t` binding from the keymap, so it is the effective binding, never a literal (keymap
  data rule, CLAUDE.md). The sort itself still doesn't change.
- **The hint bar stops offering `s`** while the list is in, or heading for, tree mode
  (`target_tree_mode()`).
- **The `?` help keeps the row, dimmed, with the reason** (`sorted by name in tree view`). This
  follows the existing rule that refused keys are "shown dimmed with the reason, never hidden"
  (`help.rs`, `Refusal`). Generalise that mechanism only as far as this needs. If it is tied
  too closely to Read-only Mode to extend cleanly, add the smallest separate "unavailable, with
  reason" marker the renderer already knows how to dim, and say which you did in the PR body.
- **Tests:**
  - an `update` test that `s` in tree mode leaves the sort unchanged and sets the notice, and
    that `s` in flat mode still cycles;
  - golden frames: the hint bar in tree mode without `s`, the help overlay in tree mode with `s`
    dimmed, and the notice after pressing `s`.

### 2. The global line at 80 columns (#111)

- **Every global key stays visible at 80×24**, `quit` and `help` included.
  - **Preferred:** the `everywhere` line wraps onto as many rows as it needs. The overlay's row
    budget (`HERE_ROW_BUDGET` in `help.rs`'s tests, currently 17 = 24 − 5 chrome − 1 footer − 1
    packed everywhere line) then drops by the extra rows. Check every `HelpContext` still fits.
    The existing test enumerates them all (`every_context`).
  - **If any context no longer fits:** keep one row and drop the lowest-value global entries
    first, keeping `quit`, `help` and `⌃R`. Choose the order deliberately and write it down.
    Either way, decide by the test, not by eye.
- At widths where the line already fits, nothing changes.
- **Tests:** update the `help_*_80` golden frames and check every changed frame by eye. Add a
  test that, for every `HelpContext` at 80×24, every entry of `help::everywhere` appears in the
  rendered overlay.

### 3. Docs, in the same PR

- DESIGN §4 (keymap/help), wherever it describes the hint bar or the help overlay's global line.
- The user guide: `site/content/docs/browsing/filter-sort.mdx` currently explains the tree-view
  sort behaviour. Make it match: what `s` does in tree view, and how to sort (`t` to flat). Fix
  anything in the help or keybindings pages that contradicts the new behaviour.
- The generated keybindings reference, regenerated with `UPDATE_DOCS=1` if it changes. Never edit
  it by hand.
- `CHANGELOG.md` `Unreleased`: a "Fixed" entry for each issue.
- This plan: mark the Status done.

## Shared rules

- Use the glossary's words, from `CONTEXT.md`.
- The core stays pure. Colours are tokens, never literals. Keybindings are data.
- Follow the keymap growth rule (DESIGN §4, ADR-0020). No new keybinding is needed here.
- No tags and no repo-settings changes. Never remove Docker containers you didn't create.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The PR
  body says `Fixes #108` and `Fixes #111`, and ends with
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Pass the body with
  `--body-file`. Edit it with `gh api`, because `gh pr edit` fails.

## Checks

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- the boundary check from CLAUDE.md
- `pnpm --dir site build`
