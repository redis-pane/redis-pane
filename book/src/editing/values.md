# Editing values

You edit a value where it is, inside the value pane. A value that reads as JSON stays visible
while you type. Every edit follows the same three steps: edit, stage, then confirm. See
[Previews and confirmation](../safety/previews.md). Read-only Mode refuses at the confirm step,
so you can try an edit on `prod` and see the command without running it.

Put the value pane in focus (`Tab`, or open a key with `→`). For a Hash, List, Set or Sorted set,
press `Enter` to put the cursor on the row you mean.

## String and JSON

| Key | Does |
| --- | --- |
| `e` | Open the inline editor on the whole value. |
| `⌃S` | Stage the edit: show the command and the change. |
| `⌃Z` `⌃Y` | Undo and redo inside the editor. |
| `Esc` | Throw the edit away. |

A value that is JSON opens pretty-printed. After `⌃S` you see the change as old text and new text
(capped to a few lines) and the command. Staging with no change just closes the editor.

The write is `SET key value KEEPTTL XX`. It keeps the key's TTL, and it only writes if the key
still exists. If the key has gone by the time you confirm, nothing is written, the key is
**not** recreated, and your text goes back into the editor so you don't lose it.

Values over 200 KB aren't edited inline. `e` says so. Opening a value in your own `$EDITOR` is
not built yet.

## Hash

| Key | Does |
| --- | --- |
| `e` | Edit the value of the field under the cursor. |
| `a` | Add a field. A two-part form, `FIELD` then `VALUE`. `Enter` moves on, `↑` moves back. |
| `d` | Remove the field under the cursor. |
| `R` | [Rename the field](rename-duplicate.md#rename-a-field-or-member). |

`Enter` stages a field edit. While you type a new field's name, `⚠ exists` appears if it matches a
field you can see, and you can't go on until it's changed. Adding never overwrites a field that
appeared in the meantime. Removing the last field warns you that the key will be deleted, because
Redis deletes an empty Hash.

## List

| Key | Does |
| --- | --- |
| `e` | Edit the element under the cursor. |
| `a` | Add an element. `Tab` switches between the head and the tail. |
| `d` | Remove the element under the cursor. |

List elements are addressed by their position. If the list changed under you so that the
element is no longer what you saw, the change is refused rather than hitting the wrong one.

## Set

| Key | Does |
| --- | --- |
| `a` | Add a member. |
| `d` | Remove the member under the cursor. |
| `R` | [Rename the member](rename-duplicate.md#rename-a-field-or-member). |

A member is its name, so there is nothing to edit: rename it, or remove it and add another.

## Sorted set

| Key | Does |
| --- | --- |
| `e` | Edit the member's score. `⌃S` stages it. |
| `a` | Add a member: its name, then its score. |
| `d` | Remove the member. |
| `R` | [Rename the member](rename-duplicate.md#rename-a-field-or-member). |

## Stream and Binary

These are read-only here. You can change their [TTL](ttl.md) and delete the whole key.

## Delete a key

`d` in the key list stages `DEL` for the Selected key. With keys marked, it deletes all of them.
See [Bulk delete](bulk-delete.md). `y` confirms.

## Changes that arrive while you're editing

An open editor is never touched by a live update. The header says `✎ editing · changed · held`,
and when you finish, you see the result. See [Live updates](../viewing/live-updates.md).
