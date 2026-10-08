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

## Build design (phase 1, 2026-10-08)

Written before the code, by the agent doing the build. Decisions 1–5 above are binding and are not
reopened here; this fills in what they leave open.

### Marks representation: a bitset, measured

`State::marks: Marks` (`state/marks.rs`), indexed by Loaded-set index and separate from `LoadedSet`
(marks are selection state, not key data; `LoadedSet::clear` is not the only renumbering point we
care about, `scan_started` is). Candidates measured on 1M indices with a scratch benchmark
(release, one thread):

| marks | bitset toggle | sorted `Vec<u32>` toggle | bitset memory | sorted memory |
|---|---|---|---|---|
| 1,000 | 7 ns | 168 ns | 125 KB | 8 KB |
| 10,000 | 2 ns | 665 ns | 125 KB | 44 KB |
| 100,000 | 3 ns | 3.4 us | 125 KB | 365 KB |
| 500,000 | 14 ns | 12 us | 125 KB | 1.3 MB |

Membership (the per-visible-row test the renderer makes) is 42-250 ns per 50 rows for the bitset and
about 1-2 us for the sorted vector. **Pick: bitset** (`Vec<u64>`, grown lazily to the highest index
marked, so a session that never marks pays nothing) with a maintained count. Toggle and membership are
O(1) and flat; memory is `Loaded/8` bytes at worst (250 KB at the 2M cap), reported by
`Marks::heap_bytes`. Enumerating marks in ascending index order (for the preview and the delete) walks
the words with `trailing_zeros`: O(Loaded/64 + marked), about 0.2-0.5 ms at 1M with 10k-500k marks,
never a per-key scan of names.

### Lifecycle

- **Survive** filter, sort, tree toggle, group expand/collapse, narrowing and every sliced rebuild job
  (M6): none of them renumbers Loaded-set indices, they only permute or subset the view over them.
  A mark on a row the filter hides stays set and is still deleted by `d`; the dialog says how many
  keys it will delete (and on `prod` the user types that count), but it cannot say how many are hidden
  (that would be O(Loaded)). Recorded as a known limit.
- **Cleared by a rescan**, in `scan_started`, on the line next to the `metadata_epoch` bump (same
  reason: the numbering changed).
- **Cleared by a completed bulk delete.** A cancelled or failed one unmarks only the keys it
  processed, so the remainder stays marked for a retry. Unmarking is guarded by the epoch the delete
  was staged under: if a rescan intervened the marks are already gone and the old indices are not used.
- **Esc** in the keys pane: first an error, then (new) an in-flight bulk delete is cancelled, then the
  value-pane pop, then (new) marks are cleared, then a running scan is cancelled. So a selection is
  never stuck, and one Esc does one thing.
- Marking a gone row is refused with a notice; rows that went gone while marked are dropped when
  `d` stages the delete (they have nothing left to delete, as single delete already refuses them).
- `Space` on a group row: notice `groups can't be marked - mark their keys`, nothing marked.

### Glyph role and readout

`Glyph::Marked`: `◆` / `+`, one column each (the table test enforces it), drawn in the pane's left
margin column (column 0 of the keys pane, which no row uses), so no column moves and a selected row
keeps its bar. Monochrome safe: it is a glyph, not a hue. A new `Glyph::Times` (`×` / `x`) is added
for `DEL × N`. The status line gains `N marked` (and, while a bulk delete runs, `deleting X of N` plus
the existing `Esc cancel`). `Space` is spelled `Space` by `key_label`.

### Mutation shape

`Mutation::DeleteKeys { keys: Vec<KeyName> }` (`key()` is `None`, like `ResetSlowlog`), staged as
`PendingMutation::DeleteKeys { indices, names, epoch, gate }`. Staging is O(marked): one name copy per
marked key, no pass over the Loaded set's names. The dialog shows the line `DEL × 2,000 keys`, a
muted guard line (`one DEL per key, in batches · Esc cancels between batches`), the first 8 names
and `… and N more` beyond that. `command_label` (error lines) is `DEL (N keys)`; the failure line
names the real failing `DEL <key>`.

**`prod` typed-count state machine** (`CountGate`): `None` (every other Environment: one `y`),
`Required` (prod, dialog just opened), `Typing { text, wrong }`.

- `Required` + `y`: Read-only Mode is checked here, as ever (refused: dialog closes with the
  notice); otherwise `Typing`. Every other key is ignored.
- `Typing`: digits append (up to 10), `⌫` removes, `Enter` submits: the exact count executes
  (through `into_command`, the one place); anything else sets `wrong` and the dialog stays. `y` and
  every other key are ignored; `Esc` dismisses at any stage. Enter, not auto-submit, because `50` is
  a prefix of `500`. The rule "only `y` confirms" is for the preview step; the typed step is
  deliberately a different gesture, which is the point of it.

### Execution in the shell

`Command::Execute { mutation: DeleteKeys, index: None }` is the existing chokepoint output. The shell
(`Shell::mutate`) routes `DeleteKeys` to `redis::mutate::delete_keys`:

- Batches of `BULK_DELETE_BATCH = 500` keys, each a `fred` pipeline of single-key `DEL`s collected
  with `try_all`. Each command is routed by its own slot (`fred` writes pipelined commands one at a
  time through the router), so a Cluster needs no grouping and never sees `CROSSSLOT`.
- Reply `1` counts as deleted, `0` as already gone. An `Err` at position `p` of a batch stops the
  run after that batch: the report carries `processed` (a contiguous prefix: whole batches plus the
  Ok run before the first error), counts for that prefix, and `extra_ok` (positions after the first
  error that did succeed, so no deleted key goes unbadged and no count lies), and
  `BulkStop::Failed { command, detail }` naming the first failing `DEL key`. The wedge check
  (`is_wedged`) runs on that error exactly as `execute_settled` does, and sets `link_lost`.
- **Cancellation:** the core's `Esc` emits `Command::CancelBulkDelete`; the shell holds a
  `CancellationToken` (`Shell::bulk_cancel`, the `scan_cancel` pattern) that `delete_keys` checks
  between batches. A cancelled run reports `BulkStop::Cancelled` and the counts so far. A batch in
  flight is allowed to finish (it is a few milliseconds; abandoning it would lose its counts).
- **Progress:** after each batch the shell sends `Msg::BulkDeleteProgress { done, total }`.
- Settlement: `Msg::MutationSettled { result: Ok(MutationOutcome::BulkDeleted(report)) }`.
  `MutationOutcome` stops being `Copy` (it now holds strings), nothing relied on it.

### Core handling of the outcome

`State::bulk: Option<BulkDelete { indices, total, done, cancelling, epoch }>` is set when `y`
produces the command and cleared on settle. For each processed position whose staged name still matches
`keys.name(index)` and whose epoch is current, the row gets `set_gone` through the same helper the
single delete uses (`mark_deleted`, factored out of `key_deleted`, which also tombstones an Open key
with its last value kept). Then the processed indices are unmarked and one message is raised: a notice
`deleted 1,998 of 2,000 keys (2 already gone)`; cancelled: `cancelled: deleted X of N ...`; failed:
the error line `DEL k: <detail> - stopped after X of N (Y deleted, Z already gone)` (R7.4).
While `bulk` is set, `d` says `a bulk delete is already running` and nothing is staged.

### Tests planned

Core unit tests per the Testing section; Docker tests call `delete_keys` directly (batch size is a
parameter, and the cancel test cancels the token from the progress callback after the first batch,
so cancellation is deterministic); perf in the M6 harness; goldens as listed.

### Phase log

- Phase 1: this note. No code.
