# filter: a substring filter, then a glob, then Esc.
#
# Shows `/` narrowing the list as you type, `⏎` applying and handing the keyboard back,
# `/` again to change it to a glob (`user:*:session`), and `Esc` in the prompt clearing it.
# Embedded in: browsing/filter-sort.
#
# Keys are from site/content/docs/reference/keybindings.md: `t` tree/flat, `/` filter,
# `⏎` apply, `Esc` clear & exit, `q` quit.
#
# poster: 9

Source "_settings.tape"
Output ../site/public/demos/filter.mp4

Require redis-pane

Hide
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 3s
Show
Sleep 1s

# Flat, so every match is a row of its own.
Type "t"
Sleep 1s

# A substring: the list narrows as you type.
Type "/"
Sleep 400ms
Type "session"
Sleep 1500ms
Enter
Sleep 1500ms

# Change it to a glob. `*` matches any run of characters.
Type "/"
Sleep 400ms
Backspace 7
Type "user:*:session"
Sleep 1800ms
Enter
Sleep 2s

# Esc in the filter prompt clears the filter and the whole Loaded set is back.
Type "/"
Sleep 600ms
Escape
Sleep 2s

Hide
Type "q"
Sleep 400ms
