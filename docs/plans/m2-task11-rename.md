# M2 task 11: Rename a key

Status: **planned.**

## Context

This is row 11 of PLAN §5, and R4.3 asks for it. The rules every task in this group follows are
in [`m2-remaining-planning.md`](m2-remaining-planning.md). Key mutations already go through one
chokepoint. Delete uses it: `d` in the keys pane stages a `DeleteKey`, the preview shows it, `y`
runs it, and a completed delete marks the row gone. Rename needs three things that path doesn't
have yet:
- a name capture;
- a check for a target that already exists;
- a key that appears where none was before.

## Decisions

1. **`R` in the keys pane**, on a key row, opens a one-line capture prefilled with the current
   name, with the cursor at the end.
   - Reuse the existing single-line capture widget that the filter, TTL and add forms already use.
   - On a group row, or with nothing selected, show a short notice and stage nothing.
   - `Esc` discards the capture. `Enter` stages the rename.
   - An empty name, or a name equal to the current one, is refused in the capture with an inline
     reason.
   - Names are bytes. Allow a binary name only if the existing captures already accept byte
     escapes; otherwise refuse binary source names with a notice, as List edit does (ADR-0017).
2. **The mutation** is `Mutation::RenameKey { key, to }`, executed as `RENAMENX key to`.
   - Reply `0` → `NotWritten::TargetExists`, a new variant with a clear dialog wording.
   - Reply `ERR no such key` → `NotWritten::KeyGone`.
   - The rename keeps the TTL; that is Redis's own semantics, and nothing is reimplemented.
3. **The preview** shows `RENAMENX <old> <new>` and `old → new`.
   - When the capture is staged, the shell runs one `EXISTS new`. The answer arrives as a message,
     because the core does no I/O. If the target exists, the dialog says so ahead of time. `y`
     still runs `RENAMENX`, which refuses atomically, so the pre-check is advice and the atomic
     command is the guard.
   - On a Cluster, if `old` and `new` hash to different slots, the dialog explains that a rename
     across slots fails with `CROSSSLOT`. `y` does nothing and only `Esc` leaves. Computing the
     slot is pure (CRC16), so it can run in the core.
4. **After a successful rename:**
   - The old row is marked gone.
   - The new name enters the Loaded set as a one-key `ScanBatch`, so the cap stays enforced only
     in `scan_batch`.
   - If the renamed key was open, the Open key follows it to the new name and re-arms through the
     normal read path. Liveness is never carried across implicitly (ADR-0006).
   - The selection moves to the new row.
5. **Read-only Mode** refuses at confirm, exactly like delete. Help and the hint bar show `R` with
   the dim and reason that mutation keys use.

## Files touched

| Area | Files |
|---|---|
| Core | `crates/core/src/mutation.rs`, `crates/core/src/update/` (keys, confirm), `crates/core/src/state/`, `crates/core/src/keymap/mod.rs`, `crates/core/src/help.rs`, `crates/core/src/render/` |
| Shell | `crates/app/src/redis/mutate.rs` (execute, plus the `EXISTS` pre-check) |
| Docs | DESIGN §4 and §6.5 |

## Testing

- **Docker:**
  - a rename lands and keeps the TTL;
  - a taken target is refused and both keys are unchanged;
  - a gone source gives `KeyGone` and creates nothing;
  - a binary name round-trips, if it is allowed;
  - on the cluster harness, a same-slot rename (hash tags `{x}a` → `{x}b`) lands, and a
    cross-slot rename is refused at the preview with nothing sent.
- **Core:**
  - capture validation;
  - the pre-check message arriving before and after `y`;
  - read-only refusal;
  - selection and Open key following the rename;
  - the old row gone, the new row present through the scan path;
  - the cap still holds when the Loaded set is full.
- **Golden frames:** the capture, all three dialog states (ok, target exists, cross-slot), and
  the hint bar.

## Build design

Written before the code, in phase 1 of the build. Where the code later disagrees, the
`## Outcome` section says so.

### Mutation and NotWritten

- `Mutation::RenameKey { key, to }`, executed as `RENAMENX key to`. `key()` returns `key` (the
  source); `command_label()` is `RENAMENX <key>` (the target is a name, safe to show, so the
  label is `RENAMENX <key> <to>`, the same as the preview).
- `NotWritten::TargetExists`, reason `"target key already exists"`. `KeyGone` is reused for
  `ERR no such key`.
- `PendingMutation::RenameKey { index, name, to, target, cross_slot }`.
  - `index` is the Loaded-set row it came from, as `DeleteKey` carries one.
  - `target: TargetCheck` is `Unchecked | Free | Taken`, the answer to the `EXISTS` pre-check.
  - `cross_slot: bool` is computed once at staging, on a Cluster only.
  - `command_text()` is `RENAMENX <old> <new>`; `guard_text()` is `only if the target is free ·
    keeps its TTL`.

### The capture

A new modal, `State::rename: Option<RenameCapture { index, from, text }>`, with its own
`Mode::Renaming` ranked beside `Filtering`. The existing single-line captures divide into two
kinds: the Open key's editor (Hash/Set add forms, TTL), which hangs off `OpenKey` and so cannot
serve a key that is not open, and the keys-pane filter, which is a state-level mode that pushes
and pops `char`s. The rename capture is the filter's shape (same keys: type, `⌫`, `Enter`, `Esc`,
`⌃`/`alt` ignored) in a small dialog drawn with `draw_confirm_box`, titled ` rename `. The text
is prefilled with the current name and the cursor sits at the end; like the filter there is no
mid-text cursor, so editing is `⌫` and typing. Pasting goes through the same `Msg::Paste` route
the filter uses.

Inline validation is live, below the field: empty (`name can't be empty`) and unchanged (`same
as the current name`). `Enter` with either does nothing but leave the reason on screen.

**Binary names.** No existing capture accepts byte escapes, so a binary name is not allowed:
`R` on a key whose name is not valid UTF-8 shows a notice and stages nothing (ADR-0017's rule
for List edit). A typed target is always valid UTF-8.

`R` is `Action::Rename`, keys-pane scoped (`pane_is_on_screen` joins the `Delete`/`Edit` group).
In the value pane it is a no-op until task 14 gives it a meaning. On a group row or with nothing
selected it shows a notice. A gone row is refused with a notice, as delete does.

### The EXISTS pre-check

The core does no I/O, so `Enter` in the capture does three things at once: closes the capture,
stages `PendingMutation::RenameKey { target: Unchecked, .. }`, and emits
`Command::CheckTarget { key: to }`. The shell runs one `EXISTS to` on the main client and sends
`Msg::TargetChecked { key: to, exists }`; on an error it sends `Msg::Failed` naming `EXISTS <to>`
(and `ConnectionLost` if `is_wedged`), and the preview simply stays `Unchecked`.

- The reply is applied only if `state.confirm` is still a `RenameKey` whose `to` equals the
  reply's key. Otherwise it is dropped: that is the case after `Esc` (nothing staged) and after
  `y` (the dialog has closed and `RENAMENX` is already on its way).
- Arriving after `y` is therefore harmless by construction, and arriving before `y` only changes
  the dialog's wording. `y` is never blocked by `Taken`: the pre-check is advice and `RENAMENX`
  is the guard (decision 3). The dialog warns, and an unchecked dialog says nothing.
- A cross-slot rename emits no pre-check; it can never run.

### Cluster slot computation

No CRC16 or key-slot function exists anywhere in the workspace (the shell leans on `fred`, and
the test harness asks the server). A new pure module `crates/core/src/slot.rs` implements
`key_slot(&[u8]) -> u16`: CRC16/XMODEM of the key, or of the hash-tag body when the key contains
`{...}` with a non-empty body, modulo 16384. It is `no_std`-level pure, so the boundary check is
untouched, and it is tested against the published vectors (`123456789` -> `0x31C3`, hence slot
12739) and hash-tag cases. The harness's `key_in_slot_of` is a server answer, so the two are
cross-checked in the Docker suite against `CLUSTER KEYSLOT`.

When `State::on_cluster()` and `key_slot(from) != key_slot(to)`, the dialog says `RENAME fails
with CROSSSLOT across slots` and `y` does nothing (the dialog stays; only `Esc` leaves), so
nothing is ever sent. This is checked in `confirm_key` before the Read-only check is even
reached, so the refusal is the same under every Environment, and the Read-only refusal still
closes the dialog as before for a same-slot rename.

### Entering the Loaded set

On `Ok(Done)` the core handles `Mutation::RenameKey` in `mutation_settled`:

1. The old row is `set_gone` (index guarded by name, as `key_deleted`).
2. The new name enters through `scan_batch(state, vec![to])`: the same function a scanned page
   goes through, so the cap is enforced in exactly that one place. When the set is full the push
   is refused, the scan state becomes `Capped` (which is true: the new key is not in the list)
   and the notice says the key is not shown. `scan_batch` is safe with no scan running: it only
   rewrites `ScanState::Running`, and it already handles a running M6 job by marking it dirty.
3. A one-key batch does not trigger `scan_batch`'s geometric rebuild for a sorted or tree view,
   so the new key could be loaded but have no row. The core therefore records
   `State::follow: Option<usize>` (the new key's index) and starts a `rebuild_list_async()` when
   the key has no row yet. `rebuild_step`, after a swap, resolves `follow`: row found -> select
   it; no row and the swap covered the index -> the filter hides it, drop the follow; the swap
   did not cover it -> start another job. Small keyspaces rebuild synchronously, so it resolves
   immediately.
4. The new key is not scanned twice by design, but a scan still running may deliver it later and
   produce a second row. `SCAN` already tolerates duplicates (the status readout says so); no
   dedup is added.
5. `scan_batch`'s own `FetchMetadata` for the visible rows fetches the new row's type and TTL.

### Open key and selection following

If the renamed key was the Open key, the Open key is retargeted to the new name (`open.name`,
`open.index`, `open.row`) and a normal `refetch()` is issued, which re-arms tracking through the
one read path. Retargeting rather than issuing a fresh open read is deliberate: the old key's
invalidation arrives as a nameless `Msg::Invalidated`, whose `refetch()` reads `open.name`; if
that still said the old name it would supersede the new read and tombstone a key that was merely
renamed. The Viewer shows the last-read bytes during that one round trip, the same as after any
write (ADR-0006: no cache, the reply is what lands). Liveness is never carried over: `refetch`
re-arms, and the header reads `consumed`/`manual` until `TrackingArmed`.

The selection moves to the new row when the Selected key was the renamed one, which is the case
for a rename started by `R`; if the reader has since moved the cursor elsewhere it is left alone
and only the Open key follows.

### Errors

Every outcome has a visible result: `TargetExists` and `KeyGone` raise an error naming
`RENAMENX old new` even when the source is not the Open key (the existing `not_written` returns
early unless the key is open, so rename is handled before it). `KeyGone` also marks the old row
gone. An `Err` goes through the existing `failed()` path.
