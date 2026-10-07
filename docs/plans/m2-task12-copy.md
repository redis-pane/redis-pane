# M2 task 12: Copy (duplicate) a key

Status: **planned.**

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
