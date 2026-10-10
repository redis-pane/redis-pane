# edit-value: edit a JSON String inline, stage it, read the command, confirm.
#
# Shows the three steps every change takes: edit in place (the value stays pretty-printed while
# you type), `⌃S` to stage (old and new text, plus the literal command), `y` to run it.
# Embedded in: editing/values.
#
# Keys are from site/content/docs/reference/keybindings.md: `→` open, `e` edit, `⌃S` stage,
# `y` confirm, `Esc` back, `q` quit.
#
# poster: 9

Source "_settings.tape"
Output ../site/public/demos/edit-value.mp4

Require redis-pane

Hide
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 3s
Type "t"
Sleep 500ms
Type "/demo:config"
Enter
Sleep 800ms
Show
Sleep 500ms

Right
Sleep 1500ms
Type "e"
Sleep 1500ms

# Change the version number on line 5.
Down 4
Right 13
Sleep 500ms
Delete 2
Type "13"
Sleep 1500ms

Ctrl+S
Sleep 3500ms

Type "y"
Sleep 3s

Hide
Type "q"
Sleep 400ms
