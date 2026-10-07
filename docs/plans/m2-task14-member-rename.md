# M2 task 14: Rename a field or member inside a key

Status: **planned.**

## Context

This is PLAN §5 row 14, and the follow-up that rows 7 and 9 pointed to. Tasks 6 (Hash, ADR-0015),
7 (Set, ADR-0016) and 9 (ZSet, ADR-0018) each deferred "a changed name is a rename" to this task.
The shared rules are in [`m2-remaining-planning.md`](m2-remaining-planning.md).

Each existing write is a guarded Lua script with a matching `NotWritten` reason, and this task
follows the same pattern. List elements are out of scope, because they are addressed by index and
editing one is already a value edit.

## Decisions

1. **`R` in the value pane**, with the cursor on a Hash field, Set member or ZSet member, opens a
   one-line capture prefilled with the current name.
   - The capture uses the existing add-form or capture widget. An empty name, or one equal to the
     current name, is refused inline.
   - Binary names follow the existing per-type rule: refuse with the same notice that type's edit
     uses.
   - If the item is still in the shown window, a name that is already present is blocked in the
     form, as the add forms do.
2. **Each rename is one guarded script.** The preview shows the effective commands and the guard,
   never the `EVAL` (R4.4).

   | Type | Effective commands | Kept |
   |---|---|---|
   | Hash | `HSETNX key new <value>` then `HDEL key old` | The field's value. On 7.4+ the field TTL is carried over too, using task 6's `HPEXPIRETIME`/`HPEXPIREAT` `pcall` pattern, and the pattern still runs on 6.2 |
   | Set | `SADD key new` then `SREM key old` | — |
   | ZSet | `ZADD key NX <score> new` then `ZREM key old` | The score |

   Each script checks first, then writes, in this order:
   - the key exists → otherwise `KeyGone`;
   - the old name exists → otherwise `FieldGone` or `MemberGone`;
   - the new name doesn't → otherwise `FieldExists` or `MemberExists`.

   It never recreates a gone key, and never ends with both names or neither.
3. **Cluster:** everything happens inside one key, so there is no cross-slot case.
4. **After success**, the value is refetched and re-armed through the one read path (ADR-0006).
   The cursor follows the new name if it is in the window.
5. **Read-only** refuses at confirm. Help and the hint bar show `R`, labelled "rename field" or
   "rename member".

## Testing

- **Docker, per type:**
  - the rename lands and keeps the value or score;
  - a new name that is already taken is refused, with nothing changed;
  - an old name that is gone is refused;
  - a key that is gone is refused, and the key isn't recreated;
  - a binary member round-trips where that type allows it;
  - Hash on 7.4+ keeps the field TTL and still runs on 6.2;
  - the rename works on the cluster harness.
- **Core:** focus gating, capture validation, the shown-duplicate block, every dialog and refusal
  wording, read-only refusal.
- **Golden frames:** the capture, the dialogs, and the hint bar for each type.
