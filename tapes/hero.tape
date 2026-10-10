# hero: filter to one key, open it, and watch it change with no keypress.
#
# Shows the CLIENT TRACKING liveness path (ADR-0006), the one thing a screenshot can't show:
# churn.py hammers `demo:live` and the open Hash follows the server with no refresh.
# Embedded in: the landing page, the docs index, the README (GIF).
#
# Render with ./tapes/render.sh hero (it owns the Redis on :6390, the seed and the XDG dirs).
# Keys are from site/content/docs/reference/keybindings.md: `t` tree/flat, `/` filter, `⏎` apply,
# `→` open, `C` copy redis-cli command, `Esc` back, `q` quit.
#
# poster: 8.5
# gif: yes

Source "_settings.tape"
Output ../site/public/demos/hero.mp4

Require redis-pane
Require python3

# The writer starts after a delay so the key is seen still before it moves. Killed at the end.
Hide
Type "(sleep 7; exec python3 ../scripts/churn.py --port 6390 --focus demo:live --focus-share 1 --rate 3 --no-delete --no-create --no-expire --duration 90 -q >/dev/null 2>&1) & CHURN=$!"
Enter
Type "clear"
Enter
# Launch hidden so the recording opens on the app, not on a half-typed command.
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Show
Sleep 800ms

# The tree is the default view; flat puts the match one row from the cursor.
Type "t"
Sleep 1s

Type "/"
Sleep 400ms
Type "demo"
Sleep 1s
Enter
Sleep 800ms

# Open it (the filter stays applied) and let the server push changes into view.
Right
Sleep 7s

# Copy a ready-to-run redis-cli command for the open key.
Type "C"
Sleep 1500ms

Escape
Sleep 700ms

# Quit and stop the writer off camera.
Hide
Type "q"
Sleep 500ms
Type "kill $CHURN"
Enter
Sleep 300ms
