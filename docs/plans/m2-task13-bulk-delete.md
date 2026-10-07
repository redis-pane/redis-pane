# M2 task 13: Multi-select and bulk delete

Status: **planned.**

## Context

[PLAN §5](../PLAN.md) row 13, R4.3, R4.6, DESIGN §6.5. "Confirmation friction scales with blast
radius": one keypress for a single delete, a typed key count for a bulk delete on `prod`.
`Space` is reserved for multi-select (DESIGN §4). Shared rules are in
[`m2-remaining-planning.md`](m2-remaining-planning.md).

The key list lives in columnar state of up to a million keys (ADR-0010, M6). Marks follow the same
rule: a parallel structure indexed by Loaded-set index, never a per-key struct.

## Decisions

1. **Marks are core state:** a set of Loaded-set indices (a bitset or a sorted `Vec<u32>`;
   measure and pick one).
   - `Space` on a key row toggles its mark and moves the cursor down.
   - `Space` on a group row does nothing and shows a notice. Marking a whole subtree at once is
     **out of scope**: it is the high-blast-radius case and needs its own decision.
   - Marks survive filtering, sorting, tree toggle and M6 rebuild jobs, because they are keyed by
     index, not row.
   - Marks are cleared by a rescan, because indices are renumbered (the same moment
     `metadata_epoch` bumps), and by a completed bulk delete.
   - `Esc` in the keys pane with marks clears them (first `Esc` only), so a selection is never
     stuck.
   - Marked rows render with a mark glyph from the glyph set (Unicode and ASCII, width-identical).
     The status bar shows `N marked`.
2. **`d` with marks stages one `Mutation::DeleteKeys { keys }`.** `d` without marks is the
   existing single-key delete, unchanged; the PLAN row says bulk is additive, not a rewrite.
   - **Preview:** `DEL` × N, the count, and the first several names with "… and N more".
   - **Friction:** on `prod`, `y` opens a typed confirmation where the user types the count
     exactly; anything else keeps the dialog open. Elsewhere, one `y`.
   - **Read-only Mode** refuses at confirm, after composing the real command, as always.
3. **Execution in the shell:**
   - Single-key `DEL`s are sent in pipelined batches (for example 500 per pipeline), so on a
     Cluster `fred` routes each key to its owner. A multi-key `DEL` would raise `CROSSSLOT`.
   - Counting the replies gives "deleted N of M; K were already gone".
   - A failing batch reports the command and how far it got (R7.4) and stops. It never retries
     silently.
   - The operation can be cancelled with `Esc` between batches (CLAUDE.md: every in-flight
     operation is cancellable). Cancelling reports how many keys were deleted.
4. **Afterwards:** every deleted index gets the gone badge, through the same path a single delete
   uses. Marks are cleared. An Open key that was deleted behaves as it does today when the open
   key is deleted.
5. **The cap and M6:** marking costs O(1) per keystroke. Building the key list for the preview
   costs O(marked), never O(Loaded set).

## Testing

- **Core:**
  - toggling, and marks surviving filter, sort, tree toggle and a sliced rebuild job;
  - marks cleared on rescan and after delete;
  - `Esc` clearing marks;
  - preview truncation;
  - the `prod` typed count, wrong-count and `Esc` paths;
  - non-`prod` one-`y`;
  - read-only refusal;
  - `d` without marks unchanged.
- **Docker:**
  - a bulk delete of 2,000 keys standalone lands and is counted;
  - some keys already gone are counted as such;
  - cancel mid-way reports a partial count;
  - on the cluster harness, keys spread across all three primaries are all deleted, with no
    `CROSSSLOT`.
- **Perf** (M6 harness): `Space` at 1M keys is under 1 ms, and a bulk-delete preview with 10,000
  marks takes a few ms.
- **Golden frames:**
  - marked rows (Unicode and ASCII);
  - the status count;
  - the dialog, below and above the truncation;
  - the `prod` typed confirmation, empty and with a wrong count;
  - the hint bar.
