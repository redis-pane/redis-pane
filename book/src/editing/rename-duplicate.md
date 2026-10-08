# Rename and duplicate

## Rename a key

Press `R` on a key in the list. A small dialog opens with the current name filled in. Edit it and
press `⏎`, and you see the command, `RENAMENX old new`. `y` runs it and `Esc` cancels.

It **never overwrites**. `RENAMENX` refuses a name that is already taken, atomically, and
`redis-pane` also checks first, so the dialog warns you before you press `y`. The key keeps its
TTL.

After a rename, the old row is badged gone, the new name appears in the list and is selected,
and if the key was open, the Viewer follows it to the new name and stays live.

Not valid on a group row, a key that is already gone, or a key whose name isn't valid text. You get
a notice instead.

## Duplicate a key

Press `D`. The dialog is prefilled with the name plus `:copy`. It runs Redis `COPY`, which needs
Redis 6.2 or newer (below that, `D` is dimmed in help and says why). The copy carries the TTL, and
like rename, never overwrites an existing key. The selection and the open key stay on the
original.

## On a Cluster

`RENAME` can't move a key between hash slots. If the two names land in different slots, the
rename dialog says `RENAME across slots fails with CROSSSLOT`, and suggests using a shared
`{tag}` in both names so they land together. `y` does nothing, and nothing is sent.

`D` across slots still works: the server refuses a cross-slot `COPY`, so `redis-pane`
copies with `DUMP` and `RESTORE` and the dialog lists those steps. It carries the TTL, and still
never overwrites.

## Rename a field or member

With the cursor on a Hash field, a Set member or a Sorted-set member, `R` renames it in place.
The dialog says `rename field` or `rename member`, and shows the key it's in.

- The new name is written before the old one is removed, so you never end up with both or
  neither.
- A Hash field keeps its value (and, on Redis 7.4 and newer, its own TTL). A Sorted-set member
  keeps its score.
- It refuses a name that is already taken. The dialog warns about names in the part of the key
  you can see, and the server's check catches the rest.
- Lists, Strings, JSON, Binary and Streams have no names to rename.

Read-only Mode refuses all of these at the confirm step.
