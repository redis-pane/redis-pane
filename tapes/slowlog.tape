# slowlog: `g s`, the entries, the detail strip, and `s` flipping recent / slowest first.
#
# The demo Redis runs with a low `slowlog-log-slower-than` (render.sh sets it, on its own
# container only) and a few Lua busy loops, so the log has slow commands, a few of them slow
# enough to be coloured as warnings. Shows the list, moving the selection with the full command in the detail
# strip, and `s` re-sorting to slowest first.
# Embedded in: server/slowlog.
#
# Keys are from site/content/docs/reference/keybindings.md: `g s` slowlog, `s` sort, `↑↓` move,
# `Esc` back, `q` quit.
#
# poster: 7

Source "_settings.tape"
Output ../site/public/demos/slowlog.mp4

Require redis-pane

Hide
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Show
Sleep 500ms

Type "g"
Sleep 600ms
Type "s"
Sleep 3s

Down 2
Sleep 1500ms
Down 2
Sleep 1500ms

# Slowest first: the Lua loops float to the top, with their durations in warning colour.
Type "s"
Sleep 2s
Up 20
Sleep 1500ms
Down
Sleep 1500ms
Down
Sleep 1500ms

Type "s"
Sleep 2500ms

Hide
Type "q"
Sleep 400ms
