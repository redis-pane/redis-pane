# social: the whole pitch in one take, for posts. Open a key and watch another client change it
# with no keypress, then make a safe edit: stage, read the exact command, confirm.
#
# Mechanics are the other tapes': `hero` (churn.py --focus, the live update) and `edit-value`
# (`e`, `⌃S`, `y`). Not embedded on any page; it lives at
# https://redis-pane.github.io/redis-pane/demos/social.mp4 for downloading and posting.
# Longer than the site's 8 to 20 s clips, so it has its own 3 MB budget (tapes/README.md).
#
# Render with ./tapes/render.sh social (it owns the Redis on :6390, the seed and the XDG dirs).
# Keys are from site/content/docs/reference/keybindings.md: `t` tree/flat, `/` filter, `⏎` apply,
# `→` open, `e` edit, `⌃S` stage, `y` confirm, `Esc` back, `q` quit.
#
# poster: 9

Source "_settings.tape"
Output ../site/public/demos/social.mp4

Require redis-pane
Require python3

# The writer starts after a delay so the key is seen still before it moves. Killed at the end.
Hide
Type "(sleep 5; exec python3 ../scripts/churn.py --port 6390 --focus demo:live --focus-share 1 --rate 2 --no-delete --no-create --no-expire --duration 90 -q >/dev/null 2>&1) & CHURN=$!"
Enter
Type "clear"
Enter
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Show
Sleep 800ms

# Part 1: filter to the key, open it, and let the server push changes into view.
Type "t"
Sleep 700ms
Type "/demo:live"
Sleep 700ms
Enter
Sleep 800ms
Right
Sleep 9s

# Part 2: back to the list, change the filter to the config key, open it and edit it.
Escape
Sleep 700ms
Type "/"
Sleep 500ms
Backspace 4
Type "config"
Sleep 500ms
Enter
Sleep 800ms
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
Sleep 1200ms

# Stage: the preview shows the old and new text and the literal command.
Ctrl+S
Sleep 4s

Type "y"
Sleep 3s

# Quit and stop the writer off camera.
Hide
Type "q"
Sleep 500ms
Type "kill $CHURN"
Enter
Sleep 300ms
