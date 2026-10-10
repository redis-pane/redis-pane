# browse: the tree, folding with Left/Right, the flat list, and the sort orders.
#
# Shows a tree group collapsing and expanding with `←`/`→` (never a toggle), `←` on a key
# jumping to its parent, `t` for the flat list, and `s` cycling the sorts while type, memory and
# TTL fill in. Embedded in: browsing/key-list, the README (GIF).
#
# Keys are from site/content/docs/reference/keybindings.md: `←`/`→` collapse/expand, `↑↓` move,
# `t` tree/flat, `s` sort, `Tab` focus, `q` quit.
#
# poster: 7
# gif: yes

Source "_settings.tape"
Output ../site/public/demos/browse.mp4

Require redis-pane

# Launch hidden so the recording opens on the app, not on a half-typed command.
Hide
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 3s
Show
Sleep 1s

# `audit:` is expanded on open. Left collapses it, and again on `cache:` to get a short list of groups.
Left
Sleep 1200ms
Down
Sleep 500ms
Left
Sleep 1200ms
Down
Sleep 800ms

# Right expands a group; Right again steps into its first child (a key row, not yet opened).
Right
Sleep 1800ms
Right
Sleep 1200ms

# Opening that key moved focus to the value pane, where `←` still drives the list: it jumps to the
# parent group, and a second `←` collapses it. `Tab` then puts focus back on the key list.
Left
Sleep 1200ms
Left
Sleep 1200ms
Tab
Sleep 800ms

# Flat list, then walk the sort orders. Type, memory and TTL are already filled in.
Type "t"
Sleep 1200ms
Type "s"
Sleep 1200ms
Type "s"
Sleep 1200ms
Type "s"
Sleep 1200ms

# Back to the tree.
Type "t"
Sleep 1200ms
Hide
Type "q"
Sleep 500ms
