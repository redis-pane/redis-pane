# Bulk delete: UX enhancements

Status: **planned, not scheduled.** Written 2026-10-08 after `0.1.0-beta.4`. The user will decide
later whether, and which parts, to build. One related change has already shipped: `unknown`
targets now get the same typed-count confirmation as `prod`
([`m2-task13-bulk-delete.md`](m2-task13-bulk-delete.md), "Follow-up").

## Today's flow (beta.4)

1. **Mark.** `Space` on a key row toggles a `◆` mark and moves the cursor down. The status bar
   shows `N marked`, and the hint bar shows `d delete N marked · Esc unmark all`.
   - Marks survive filtering, sorting, tree toggling and background rebuilds.
   - `Esc` clears them, and so does a rescan.
   - `Space` on a group row shows a notice and marks nothing.
2. **Stage.** `d` opens a single dialog. It shows `DEL × N keys`, the guard line, the first eight
   names and `… and N more`.
3. **Confirm.**
   - On `local` and `staging`, one `y` confirms.
   - On `prod` and `unknown`, `y` opens a field where you type the count and press `⏎`. A wrong
     count keeps the dialog open.
   - Read-only Mode refuses at `y`, after the dialog has been composed.
4. **Run.** Keys are deleted one `DEL` each, pipelined in batches of 500. The status bar shows
   `deleting X of N · Esc cancel`.
   - `Esc` stops the run at the next batch boundary.
   - Deleted rows get the gone badge, and keys that were not processed stay marked for a retry.
   - The result reads "deleted N of M; K were already gone".

## Gaps

1. **Marking at scale is tedious.** The real task is usually "everything under `session:`" or
   "everything this filter shows", and both mean one keypress per key.
2. **Hidden marks surprise.** A marked key the filter hides is still deleted. The count includes
   it, but nothing says it is off screen.
3. **The preview can't scroll** past eight names, and a large delete is where that matters most.
4. **Friction doesn't scale with count off `prod`/`unknown`.** 50,000 keys on `staging` is one `y`.

## Proposed changes

| # | Change | Behaviour | Risk |
|---|---|---|---|
| A | Mark a group | `Space` on a group row marks every **loaded** key under it, and unmarks them if all are already marked. The status bar says `+1,204 marked under user:8812:` | One key can mark 100k. Ship together with D and F |
| B | Mark the filter's matches | One key in the keys pane (proposed `*`, view-scoped per the keymap growth rule) marks every row the filter currently shows. Without a filter it refuses with a notice, so it can never mean "the whole Loaded set". This is "delete by pattern" without `KEYS`, over what `SCAN` loaded | Same as A |
| C | Scrollable preview | In the confirm dialog, `j`/`k` and `PgUp`/`PgDn` scroll the full name list. It is O(marked), never O(Loaded) | Low |
| D | Say what's hidden | The dialog reads `DEL × 12 keys (3 not shown by the current filter)`. A `v` in the dialog lists the hidden ones | Low |
| E | Typed count on `unknown` | **Done** (2026-10-08) | — |
| F | Friction scales with count | Above a threshold (proposed 1,000 keys, a named constant, possibly configurable later), the typed count applies on every Environment | Low |

## Rules any of these must keep

- **Loaded keys only.** A group or filter mark covers what `SCAN` has loaded, never the server's
  whole keyspace. The dialog says so, for example: `1,204 loaded keys under user:8812:; the scan
  is 40% done`, so a partial scan is never mistaken for "all of them".
  - Deleting a server-side pattern exhaustively is a different feature, a server-side `SCAN` +
    `DEL` job. It needs its own design and its own ADR, because its blast radius is not visible
    before it runs.
- **Marks stay columnar.** They remain the bitset (`state/marks.rs`). Group and filter marking
  walk the shown rows once, O(rows) per keystroke, never per-key structs. The perf harness gains
  a "mark a 100k-row group" case at 1M keys, which must stay under one frame or become a sliced
  job like M6's rebuilds.
- **One path.** It is still one `Mutation::DeleteKeys` through the chokepoint, with Read-only
  Mode refusing at confirm after composing.
- **Cancellable,** exactly as today.

## Suggested grouping, when scheduled

1. **Task 1: A + C + D + F.** Group marking, plus the safety pieces that make it reasonable at
   scale.
2. **Task 2: B.** Filter marking, after the user has tried task 1.

Each task follows the usual process: its own plan doc, one subagent, a PR merged on green, and
golden frames for every new dialog line and status readout.
