# edit-members: add a Hash field, change a Sorted-set score, remove a List element.
#
# Each goes through its own preview: the literal command (HSET, ZADD ... XX, LREM-style guarded
# script shown as the command it performs) and `y` to run it.
# Embedded in: editing/values.
#
# Keys are from site/content/docs/reference/keybindings.md: `→` open, `a` add, `e` edit, `d`
# remove, `⏎` stage / put the cursor on a row, `↓` move, `Tab` focus, `⌃S` stage, `y` confirm.
#
# poster: 7

Source "_settings.tape"
Output ../site/public/demos/edit-members.mp4

Require redis-pane

Hide
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 3s
Type "t"
Sleep 500ms
Type "/demo:m:"
Enter
Sleep 800ms
Show
Sleep 500ms

# Hash: add a field (name, then value), read the preview, confirm.
Right
Sleep 1200ms
Type "a"
Sleep 600ms
Type "team"
Enter
Sleep 400ms
Type "platform"
Sleep 600ms
Enter
Sleep 1700ms
Type "y"
Sleep 1100ms

# List: put the cursor on the second element and remove it.
Tab
Down
Right
Sleep 700ms
Enter
Down
Sleep 400ms
Type "d"
Sleep 1700ms
Type "y"
Sleep 1100ms

# Sorted set: change a member's score.
Tab
Down
Right
Sleep 700ms
Enter
Down
Sleep 400ms
Type "e"
Sleep 800ms
Delete 4
Type "9500"
Sleep 600ms
Ctrl+S
Sleep 1700ms
Type "y"
Sleep 1700ms

Hide
Type "q"
Sleep 400ms
