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
