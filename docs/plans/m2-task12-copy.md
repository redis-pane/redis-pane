# M2 task 12: Copy (duplicate) a key

Status: **in progress** (branch `m2-copy`).

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
