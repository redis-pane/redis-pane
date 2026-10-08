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

## Build design

Written before the code (phase 1). Where the code later disagrees, `## Outcome` says so.

### Reuse from tasks 11 and 12

- **The capture.** `RenameCapture` (`state/rename.rs`), `State::rename`, `Mode::Renaming`, typing,
  `⌫`, paste, `Esc`, `Enter` and `HelpContext::Rename(kind)` are reused unchanged. `NameKind` gains
  three variants, `Field`, `SetMember` and `ZSetMember`, because the title, the notices and the
  mutation differ per type while the typing is shared. Titles: ` rename field `, ` rename member `
  (both members). `verb()` is `rename field` / `rename member`, `key_label()` stays `R`.
- **What the capture carries.** `RenameCapture` gains `owner: Option<KeyName>`: the Open key the
  field or member lives in (`None` for a key-level rename and copy). For a field or member,
  `from` is the old name's bytes and `index` is the value cursor row it was started from. The
  prefill is the old name; the capture is opened only for a name that is valid UTF-8.
- **Validation lives on `State`, not the capture.** The shown-duplicate block needs the Open key's
  fetched window, which the capture does not hold, so `State::rename_problem()` replaces
  `capture.problem()` at its two call sites (the overlay and `stage_name`). It returns the
  capture's `Empty` / `Unchanged`, then a new `RenameProblem::Shown` (`already in this hash` /
  `set` / `zset`) when the typed bytes equal another name in the shown window. A name taken
  outside the window is the script's job, as with the add forms.
- **Plugging in.** `Action::Rename` already reaches the value pane and is a no-op there
  (`update/mod.rs`); it now calls `begin_member_rename`. Focus gating copies `delete_value_row`:
  the keys-pane arm of the dispatch is unchanged, and the value-pane half runs the ladder:
  nothing open → `nothing to rename here`; gone key → `gone — nothing to rename`; an edit under way
  → `still saving the last edit`; Hash/Set/ZSet with no cursor → `Enter to pick a field` /
  `member`; List, String, JSON, Binary, Stream → `rename: only a hash field, set member or zset
  member can be renamed` and nothing is staged.
- **Hooks.** `PendingMutation::target_mut` is not used (no `EXISTS` pre-check: the shown-duplicate
  block is the advice and the script is the guard). `blocks_confirm` stays false. `command_lines`
  gains the three two-line previews. The three new variants join the exhaustive matches in
  `update/editor.rs` (`staged_edit_found_key_gone`), `into_command`, `command_text`,
  `guard_text`, `render::confirm_overlay`, and `update/confirm.rs`'s `nothing_to_remove`.

### Mutations

Three variants, not one with a type tag, because every existing write is one variant per type and
every exhaustive match then forces a per-type decision:

- `Mutation::RenameHashField { key, field, to }`
- `Mutation::RenameSetMember { key, member, to }`
- `Mutation::RenameZSetMember { key, member, to }`

All three carry `Vec<u8>` names. Neither value nor score is carried: the script reads it
server-side in the same atomic step, so a value changed under the dialog is kept, not clobbered,
and a 200KB field is never held in the core. `PendingMutation` has the same three variants with
`name: KeyName` (the Open key), the old name and `to`. `key()` returns the owning key; the shell
`execute` already keys everything off `mutation.key()`.

Labels for an error line (R7.4), following each type's existing add label so a member's bytes stay
out of an error line while a Hash field's name is shown as in `HSET`/`HDEL`:
`HSETNX/HDEL <key> <old> <new>`, `SADD/SREM <key>`, `ZADD NX/ZREM <key>`.

`NotWritten` reuses `KeyGone`, `FieldGone`, `FieldExists`, `MemberGone` and `MemberExists`. Set's
old-gone is `MemberGone` (not a new variant); its new-taken is `MemberExists`. No new variant.

### The three guarded scripts

One `EVAL` each, the key only via `KEYS[1]`, names via `ARGV` (binary-safe). The check order is
fixed and mirrors the spec: key exists, old exists, new absent, write the new name, delete the old.
Returns: `-1` key gone, `-2` old name gone, `-3` new name taken, `1` renamed.

```lua
-- Hash
if redis.call('EXISTS', KEYS[1]) == 0 then return -1 end
if redis.call('HEXISTS', KEYS[1], ARGV[1]) == 0 then return -2 end
if redis.call('HEXISTS', KEYS[1], ARGV[2]) == 1 then return -3 end
local value = redis.call('HGET', KEYS[1], ARGV[1])
local ttl = redis.pcall('HPEXPIRETIME', KEYS[1], 'FIELDS', 1, ARGV[1])
redis.call('HSETNX', KEYS[1], ARGV[2], value)
redis.call('HDEL', KEYS[1], ARGV[1])
if type(ttl) == 'table' and not ttl.err and ttl[1] and tonumber(ttl[1]) and tonumber(ttl[1]) > 0 then
  redis.call('HPEXPIREAT', KEYS[1], ttl[1], 'FIELDS', 1, ARGV[2])
end
return 1
-- Set
if redis.call('EXISTS', KEYS[1]) == 0 then return -1 end
if redis.call('SISMEMBER', KEYS[1], ARGV[1]) == 0 then return -2 end
if redis.call('SISMEMBER', KEYS[1], ARGV[2]) == 1 then return -3 end
redis.call('SADD', KEYS[1], ARGV[2])
redis.call('SREM', KEYS[1], ARGV[1])
return 1
-- ZSet
if redis.call('EXISTS', KEYS[1]) == 0 then return -1 end
local score = redis.call('ZSCORE', KEYS[1], ARGV[1])
if score == false then return -2 end
if redis.call('ZSCORE', KEYS[1], ARGV[2]) ~= false then return -3 end
redis.call('ZADD', KEYS[1], 'NX', score, ARGV[2])
redis.call('ZREM', KEYS[1], ARGV[1])
return 1
```

The checks are `HEXISTS` / `SISMEMBER` / `ZSCORE` rather than relying on `HSETNX` / `SADD` / `ZADD
NX` to refuse, because those cannot tell "old gone" from "new taken" afterwards, and a script
cannot half-undo. The write commands are still the `NX` forms, so a future edit to the checks
cannot turn the script into an overwrite. Writing the new name before deleting the old one means no
interleaving ends with neither, and the script is atomic, so none ends with both. It cannot
recreate a gone key, because the first line returns before any write. Renaming the only member
leaves the key non-empty throughout (the new name is added before the old one is removed), so the
key is never deleted by the `HDEL` / `SREM` / `ZREM`.

- **Hash field TTL.** `HPEXPIRETIME` is read before the write, with `redis.pcall`, and
  `HPEXPIREAT` reapplies it to the new name, exactly ADR-0015's pattern. On 7.4+ the field TTL moves
  with the name. On 6.2 (and anything below 7.4) the `pcall` returns an error table, the `type`
  check skips the reapply, and the rename still lands with no field TTL, which is correct because
  there is none. A field whose TTL has passed is not visible to `HEXISTS`, so it reads as old
  gone. `HGET` returns a binary-safe string, so a binary value survives.
- **ZSet score carry.** `ZSCORE` returns the score as a string with enough precision to round-trip a
  double (`%.17g`), and `ZADD` accepts it back, including `inf` and `-inf`. Reading it server-side
  also means the score is whatever it is at write time.
- **Cluster.** Everything touches `KEYS[1]` only, so there is no cross-slot case (decision 3).

### Binary names, per type (mirror each type's existing edit rule)

- **Hash field:** refused when the field *name* is not UTF-8, with the notice `binary fields
  aren't editable here yet` (the wording `for_hash_field` uses). `for_hash_field` also refuses on a
  binary *value*, because it has to put the value in a text buffer. Rename never puts the value in
  a buffer (the script copies it), so refusing on the value would be a restriction with no reason;
  rename accepts a binary value. This is a deliberate narrowing of "mirror", noted again in the
  Outcome.
- **Set member:** refused when not UTF-8, `binary members aren't editable here yet` (the wording
  `open_editor` gives a Set).
- **ZSet member:** `e` on a ZSet does not refuse a binary member (the score edit never types the
  member), but a rename has to prefill the member as text, so it refuses with the same Set wording.
  A typed new name is always UTF-8, so a *new* name is never binary.

### Shown-duplicate block

While the capture is open, `rename_problem()` compares the typed bytes with every other name in
the fetched window: `PairValue::pairs` fields, `SetValue::members`, `ScoredValue::entries`. Equal
to the old name is `Unchanged` (checked first), equal to another is `Shown`. `Enter` with a
problem leaves the reason on screen, like the key rename. Empty is refused for all three types,
including Set and ZSet where Redis would allow it: the decision text says an empty name is refused
inline, and an empty member is never what a rename means.

### Stage, preview, confirm

`Enter` re-checks that the Open key is still `owner` and that the old name is still at `index` in
the window (a refetch can reorder a Hash or Set under the capture). If not, a notice `rename
field: the value changed — press R again` and nothing is staged, the same guard `stage_name` has
for a rescan. Otherwise it stages the `PendingMutation`; no command is emitted.

The preview is `command_lines()` (two lines), the muted guard line, then the one fact that changes:

- Hash: `HSETNX key new <value>` / `HDEL key old`; guard `only if the field still exists · new
  name must be free · keeps its value and TTL`; then `old → new`.
- Set: `SADD key new` / `SREM key old`; guard `only if the member still exists · new name must be
  free`.
- ZSet: `ZADD key NX <score> new` / `ZREM key old`; guard `... · keeps its score`.

`<value>` and `<score>` are literal placeholders, the same device copy's `<payload>` uses; the
guard line is the R4.4 description of the script and the dialog never shows `EVAL`. Read-only Mode
refuses at `y` after the preview is composed, as for every other write (DESIGN §6.5).

### After success: refetch, re-arm, cursor follow

`mutation_settled` → `write_landed(key)` is the existing path: it refetches through the one read
path, which re-arms tracking. The three renames add one step before it: `OpenKey::follow:
Option<Vec<u8>>` is set to the new name. When the refetch reply applies (`value_loaded`, the
own-write branch), the core looks the name up in the new window; found, `cursor` moves to its row
and `offset` follows via the same `scrolled_to_selection` the cursor movement uses; not found
(outside the window, or the key changed), the cursor stays clamped where it was. The `follow` is
dropped either way, so it can never fire on a later read. No value is cached: the name is only a
target for the cursor.

A `KeyGone` refusal is handled as the add and edit guards do: the Open key is tombstoned
(`not_written`'s existing `KeyGone` arm). `FieldGone` / `MemberGone` / `FieldExists` /
`MemberExists` refuse with an error line naming the command, and refetch so the reader sees what
the server has now.

### Phase log

(Appended as the phases land.)

- **Phase 2 (core).** Landed as designed. Notes:
  - `State::rename_problem()` replaced `capture.problem()` in the overlay and in staging; the
    capture's own `problem()` stays for the empty/unchanged half.
  - `OpenKey::follow` and `update/viewer.rs::follow_renamed` implement the cursor follow.
  - The Hash field rename accepts a binary *value* (see "Binary names").
  - Mutation labels are `HSETNX/HDEL <key> <old> <new>` (the Hash field names are shown, as in
    `HSET`/`HDEL`), `SADD/SREM <key>`, `ZADD NX/ZREM <key>`.
  - The shell has a stub (the three mutations error) so the workspace compiles at this commit.
  - Goldens: only the four help overlays that list value-pane rows (`help_value_{hash,set,zset}_
    cursor_{80,130}`, `help_read_only_dimmed_{80,130}`) changed, each gaining `R rename field` /
    `R rename member`. In the 130-column ones the overlay's own bottom hint bar also loses its
    trailing `↑↓ jk move`, which the new row pushes off (the bar stops at the first row that does
    not fit; the keys pane's `R` made the same trade in task 11). No browser, dialog or other
    hint-bar frame moved.
