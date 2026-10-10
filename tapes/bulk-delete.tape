# bulk-delete: mark keys, preview the DEL, type the count on prod, watch it run.
#
# Shows `Space` marking a run of keys (◆, `N marked`), `d` staging `DEL × N keys` with the first
# names, `y` opening the count prompt that `prod` requires, a typed count, and the
# `deleting X of N` progress. Read-only Mode (the `prod` default) is lifted with `⌃R` first.
# Embedded in: editing/bulk-delete.
#
# The `prod` Profile comes from render.sh's temp config. ~2000 keys under `tmp:bulk:` are seeded
# by render.sh. A hidden `CLIENT PAUSE` holds writes for a moment so the progress line is on screen
# long enough to read: a local Redis deletes 40 keys faster than a frame.
# Keys are from site/content/docs/reference/keybindings.md: `t` tree/flat, `/` filter, `⏎` apply,
# `Space` mark, `d` delete, `y` confirm, `⌃R` read-only, `q` quit.
#
# poster: 10

Source "_settings.tape"
Output ../site/public/demos/bulk-delete.mp4

Require redis-pane

Hide
Type "(sleep 10; docker exec redis-pane-demo redis-cli CLIENT PAUSE 8000 WRITE >/dev/null) & P=$!"
Enter
Type "clear"
Enter
Type "redis-pane prod"
Enter
Sleep 3s
Type "t"
Sleep 500ms
Type "/tmp:bulk"
Enter
Sleep 800ms
Ctrl+R
Sleep 500ms
Show
Sleep 1s

# Hold Space: each press marks the Selected key and moves down.
Space@70ms 40
Sleep 1s

Type "d"
Sleep 2500ms

# On prod the first y asks for the count instead of running.
Type "y"
Sleep 1500ms
Type "40"
Sleep 800ms
Enter
Sleep 5s

Hide
Type "q"
Sleep 400ms
Type "kill $P 2>/dev/null"
Enter
Sleep 300ms
