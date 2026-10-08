# M2 task 12: Copy (duplicate) a key

Status: **done.**

## Context

[PLAN §5](../PLAN.md) row 12, R4.3. Shared rules are in
[`m2-remaining-planning.md`](m2-remaining-planning.md). This task builds on task 11 (rename) and
reuses its name capture, the existence pre-check, `NotWritten::TargetExists`, and the path by
which a new key enters the key list through `scan_batch`.

## Decisions

1. **`D` (duplicate) in the keys pane** opens the same capture, prefilled with the current name
   plus a suffix (for example `:copy`).
   - The key is `D` because `c` is the clipboard copy; the user decided this on 2026-10-08.
   - Help and the hint bar label it "duplicate key (COPY)".
2. **The mutation is `Mutation::CopyKey { key, to }`,** executed as `COPY key to` without
   `REPLACE` or `DB`.
   - `COPY` returns 0 if `to` exists, which maps to `NotWritten::TargetExists`.
   - If the source is gone, it maps to `NotWritten::KeyGone`.
   - The TTL carries over by Redis's own `COPY` semantics (PLAN row 12: not reimplemented).
   - `COPY` needs Redis 6.2 or later, and the floor is 6.0 (ADR-0007). On 6.0, `D` is disabled in
     help with the reason "needs Redis 6.2". The core reads the version probed at connect, as
     sharded Pub/Sub does.
3. **Cluster, across slots.** `COPY` refuses across slots, so the dialog says the copy runs as
   `DUMP` + `RESTORE` and shows both commands.
   - On `y`, the shell runs `DUMP src` and `PTTL src`, then `RESTORE dst <ttl-or-0> <payload>`
     without `REPLACE`.
   - `BUSYKEY` maps to `TargetExists`. A nil `DUMP` maps to `KeyGone`.
   - The payload never reaches the core or the screen.
   - A key that expires between `PTTL` and `RESTORE` gets a TTL of 1 ms, not 0 (which would mean
     no expiry). Record the rule and test it.
4. **After success,** the new key enters the list through the scan path. The selection and the
   Open key stay on the source, because a duplicate is a side effect, not a move.
5. **Move-across-db stays parked** (PLAN §5), and it doesn't exist on a Cluster at all.

## Testing

- **Docker, standalone:**
  - a copy lands with the same value and TTL for every type;
  - a taken target is refused and left unchanged;
  - a gone source refuses and creates nothing;
  - on Redis 6.0 or 6.1 (if the harness has an image), `D` is gated; otherwise unit-test the gate.
- **Docker, cluster harness:**
  - a same-slot copy goes through `COPY`;
  - a cross-slot copy goes through `DUMP` + `RESTORE`, with the value and TTL preserved;
  - a cross-slot copy onto an existing target refuses with `TargetExists`;
  - a cross-slot copy of a key with no TTL stays without one.
- **Core:** the capture prefill; the dialog in all three states (plain, target exists,
  cross-slot fallback); read-only refusal; the selection staying on the source.
- **Golden frames:** the capture, the dialogs, and the hint bar.

## Build design

Written before the code (phase 1). Where the code later disagrees, `## Outcome` says so.

### Generalised from rename (task 11)

- **The capture.** `RenameCapture` gains a `NameKind { Rename, Copy }` and keeps its type name,
  `State::rename`, `Mode::Renaming` and `HelpContext::Rename`: they mean "the one-line name
  capture", and renaming them would churn every rename test for no behavioural gain. The kind
  drives the three things that differ: the prefill (`Rename`: the current name; `Copy`: the name
  plus `:copy`, cursor at the end), the title (` rename ` / ` duplicate key (COPY) `) and the
  inline reason for an unchanged name. Typing, `⌫`, paste, `Esc`, `Enter`, the empty/unchanged
  validation and the "rescan renumbered the list" guard are shared; `begin_rename` and
  `begin_copy` are two thin entries over one `begin_name_capture(kind)`, and `stage_rename`
  becomes `stage_name`, which builds either `PendingMutation`.
- **The target check.** `TargetCheck`, `Command::CheckTarget` and `Msg::TargetChecked` are used
  unchanged. `target_checked` matches a staged `RenameKey` *or* `CopyKey` by its `to`, through one
  accessor on `PendingMutation`. `NotWritten::TargetExists` is reused: its reason, "target key
  already exists", fits copy.
- **The cross-slot flag.** `cross_slot: bool` becomes `slots: SlotPath`:
  `SameSlot | CrossSlotRefused | CrossSlotFallback`. Rename yields `SameSlot` or
  `CrossSlotRefused`; copy yields `SameSlot` or `CrossSlotFallback`. Only `CrossSlotRefused`
  blocks `y` (`PendingMutation::blocks_confirm`), so `confirm_key` and the dialog ask one question
  instead of matching on `RenameKey`. A non-Cluster is always `SameSlot`.
- **Inserting a new name.** The first half of `key_renamed`'s tail (push the name through
  `scan_batch`, and start a rebuild when the new key has no row yet) becomes
  `insert_loaded_key`. Rename adds its `follow`/`settle_follow` and Open-key retarget on top;
  copy calls only the helper. The sorted/tree rebuild keeps the selection on its key
  (`relocate_after_rows_changed`), so the selection stays on the source with no extra work.

### Copy-specific

- `Mutation::CopyKey { key, to }`, label `COPY key to`. `PendingMutation::CopyKey { index, name,
  to, target, slots }`, command text `COPY src dst`, guard line `only if the new name is free ·
  carries its TTL`. On `CrossSlotFallback` the dialog lists the three commands (`DUMP src`,
  `PTTL src`, `RESTORE dst <ttl> <payload>`), the payload being a literal placeholder.
- The shell, not the `Mutation`, picks the path: on a Cluster it compares
  `slot::key_slot(src)` with `key_slot(dst)`, the same function the core used for the preview, so
  the two cannot disagree.
- **The selection and the Open key stay on the source** (decision 4): no `follow`, no gone mark,
  no retarget. A `KeyGone` outcome marks the *source* row gone (and tombstones the source if it
  is open), as rename does.
- **The version gate.** `COPY` is Redis 6.2. `State::copy_available()` is
  `server_version().is_none_or(|v| v >= 6.2.0)`, in the same unknown-is-allowed shape as
  `sharded_available`. Below it, `D` shows the notice `duplicate: needs Redis 6.2` and stages
  nothing, and the help/hint row is dimmed with the reason `needs Redis 6.2`. That needs
  `Refusal` to carry a cause other than Read-only Mode, so it gains a `Requires(&str)` variant;
  the version gate wins over Read-only dimming. This also covers a Cluster of old nodes, since
  the gate is on the probed version, and the `DUMP`/`RESTORE` fallback is **not** offered as a
  workaround on 6.0/6.1: the gate is on `D` as a whole.

### The DUMP + PTTL + RESTORE fallback (Cluster, across slots)

1. `DUMP src` (routed to src's slot). A nil reply -> `NotWritten::KeyGone`; nothing is written.
2. `PTTL src`. The reply picks the `RESTORE` ttl argument (milliseconds, relative, no `ABSTTL`):
   - `-2` (key gone between `DUMP` and `PTTL`) -> `KeyGone`; nothing is written.
   - `-1` (no expiry) -> `0`, which `RESTORE` reads as "no expiry".
   - `n >= 1` -> `n`.
   - `0` (it expired between the two reads, so `PTTL` reported 0 ms left) -> `1`. A `0` would
     *remove* the expiry and resurrect a key that was about to die as a permanent one; 1 ms
     lets it expire immediately, which is what it was doing anyway. The rule is
     `restore_ttl(pttl)`, a pure function with unit tests: `-2 -> KeyGone`, `-1 -> 0`,
     `0 -> 1`, `n -> n`.
3. `RESTORE dst ttl payload`, **no `REPLACE`**. `BUSYKEY` -> `TargetExists`. Any other error
   propagates as a failure naming the command.

`DUMP` and `PTTL` are not atomic with each other; the 1 ms rule is the only place that gap can
change a result. The payload lives only in the shell's task and is dropped after `RESTORE`; it
never reaches `Msg`, the core, a log or the screen. The same-slot path is one `COPY src dst`:
reply `0` -> `TargetExists`; an `ERR no such key` or a `0`-with-missing-source is `KeyGone`.
A `COPY` that replies 0 is ambiguous (target taken, or source gone), so the shell
disambiguates with one `EXISTS src` before reporting.

### Phase log

- **Phase 2 (core).** Landed as designed (`NameKind`, `SlotPath`, `insert_loaded_key`,
  `key_copied`, `State::copy_available`, `Action::Duplicate` on `D`, `Refusal` gained a `needs`
  cause). Deviations:
  - `HelpContext::Rename` became `Rename(NameKind)` so help and the border title say
    "duplicate key (COPY)" for a copy; the type names `RenameCapture`/`Mode::Renaming` stayed.
  - Error lines for a Cluster cross-slot copy name `DUMP/RESTORE src dst` rather than `COPY`
    (`settled_label`), since that is what ran.
  - Six `help_*` goldens plus the two `help_disconnected_*` gained the `D duplicate key (COPY)`
    row (the help overlay lists every keys-pane binding). The row is ranked after `rename`, so
    no hint-bar or browser frame moved.
  - The shell has a stub (`CopyKey` errors) so the workspace compiles at this commit.
- **Phase 3 (shell).** `redis::mutate::copy_key` (`COPY` / `DUMP`+`PTTL`+`RESTORE`) and the pure
  `restore_ttl`, wired into `execute`; the existing `execute_settled` wedge handling and the
  `CheckTarget` shell path apply unchanged. A `COPY` reply of `0` is disambiguated with one
  `EXISTS src` (target taken vs source gone). `restore_ttl` also maps any `PTTL` below -2 to `0`
  (defensive; the server never sends it). No deviations.
- **Phase 4 (tests).** Docker, `mod copy` in `crates/app/tests/integration.rs` (11 tests):
  every type (string, hash, list, set, zset, stream) lands with equal contents and a carried TTL;
  no-TTL stays no-TTL; taken target refused with both keys unchanged; gone source creates
  nothing; binary names with decoys; Cluster same-slot `COPY` (plus taken and gone); Cluster
  cross-slot fallback with value and TTL kept (and the server really refuses plain `COPY` there);
  cross-slot no-TTL; cross-slot onto an existing target; cross-slot gone source. **Deviation:**
  the 6.0/6.1 version gate is unit-tested only. No 6.0/6.1 image is in the suite and
  `docker pull redis:6.0-alpine` hangs in this environment; the Docker test instead pins that a
  real 6.2 probe leaves `D` available and the server accepts `COPY`. Core: 20 new tests in
  `update/rename.rs`; shell: 2 for `restore_ttl`. Golden: eight new frames (copy capture,
  typed, hint bar; dialog plain, target exists, cross-slot fallback, read-only) plus a test that
  the help row is dimmed `needs Redis 6.2` below 6.2. Only help goldens changed (phase 2).

## Outcome

Status: **done.** `D` in the keys pane duplicates the Selected key through the one mutation path.

**Delivered, as designed:** the shared name capture (`NameKind::Copy`, prefilled `<name>:copy`);
`Mutation::CopyKey` / `PendingMutation::CopyKey`; the `EXISTS` pre-check shared with rename;
`SlotPath { SameSlot, CrossSlotRefused, CrossSlotFallback }` replacing rename's `cross_slot`
bool; `COPY` standalone and same-slot; `DUMP` + `PTTL` + `RESTORE` (no `REPLACE`) across slots,
with the TTL rule (`-1` -> 0, `-2` -> `KeyGone`, expired-in-between `0` -> 1 ms) in the pure,
tested `restore_ttl`; the 6.2 version gate (`State::copy_available`, unknown allowed); Read-only
refusal at confirm; the new key inserted via `scan_batch` with the selection and Open key left on
the source.

**Deviations from the plan text:**
- **`HelpContext::Rename` became `Rename(NameKind)`** and `Refusal` gained a `needs` cause so a
  help row can be dimmed for a reason other than Read-only Mode. The version gate wins over
  Read-only dimming (a key that does nothing at all has no preview).
- **Cluster error lines name `DUMP/RESTORE src dst`** instead of `COPY`, because that is what ran.
- **The 6.0/6.1 gate is unit-tested, not Docker-tested:** no such image is in the suite and
  `docker pull redis:6.0-alpine` hung here. A Docker test pins the 6.2 side.
- **The shell, not `Mutation::CopyKey`, chooses COPY vs DUMP/RESTORE,** from
  `client.is_clustered()` and `slot::key_slot`, the same function the core used for the preview.
- **A `COPY` reply of 0 costs one `EXISTS src`** to tell a taken target from a gone source.
- **Only the help goldens changed** (six `help_*` plus two `help_disconnected_*`, each gaining the
  `D duplicate key (COPY)` row). Eight new frames were added.

**Known limits:** as with rename, a scan still running may deliver the new name again, and a new
name hidden by the filter or a collapsed group is loaded but not shown. `DUMP` and `PTTL` are two
reads, not one atomic step; the 1 ms rule is the only place that gap can change a result. A
Cluster copy is not atomic across the two nodes either, but it only ever *adds* the target.

**Numbers:** core lib tests 1066 -> 1085, golden 267 -> 275, app unit 94 -> 96, Docker suite
162 -> 173; fmt, clippy `-D warnings`, `cargo test --workspace` and the boundary check clean.

**For task 14 (member rename) and task 13 (bulk delete):**
- The name capture is `RenameCapture` in `state/rename.rs` with a `kind: NameKind`; task 14 can
  add a `NameKind::Member` (or a sibling struct) and reuse typing/validation. `State::rename`,
  `Mode::Renaming` and `HelpContext::Rename(kind)` are keyed by kind. `RenameProblem::reason`
  takes the kind.
- `insert_loaded_key` (`update/rename.rs`) is the helper for "a key appeared": it goes through
  `scan_batch` and starts a rebuild when the key has no row. Task 13 does not need it.
- `PendingMutation::blocks_confirm`, `command_lines` and `target_mut` are the generic hooks for a
  dialog that cannot run, runs as several commands, or carries an `EXISTS` answer.
- `Refusal` now has `needs: Option<&str>` for non-Read-only dimming; `settled_label` in
  `update/confirm.rs` is where an error line's command name can differ from `command_label`.
- Task 13 must handle `PendingMutation::CopyKey`/`RenameKey` only through the existing exhaustive
  matches (`editor.rs` guard, `into_command`); `D` and `Space` do not collide.
