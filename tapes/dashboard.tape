# dashboard: `g d`, the tiles filling in, ops/sec moving under churn, Enter for the raw INFO.
#
# Shows the six INFO tiles refreshing every two seconds while another client writes (churn.py is
# the only traffic, so the Ops/sec tile and its sparkline move), focus moving between tiles, and
# `⏎` expanding one tile into the raw INFO section behind it, `Esc` closing it.
# Embedded in: server/dashboard.
#
# Keys are from site/content/docs/reference/keybindings.md: `g d` dashboard, `←→↑↓ hjkl` move
# focus, `⏎` expand tile, `Esc` back, `q` quit.
#
# poster: 8

Source "_settings.tape"
Output ../site/public/demos/dashboard.mp4

Require redis-pane
Require python3

# A hidden writer, so ops/sec has something to count: churn at 40 op/s. It stops itself
# after --duration seconds and is killed at the end of the tape.
Hide
Type "python3 ../scripts/churn.py --port 6390 --rate 40 --duration 60 -q >/dev/null 2>&1 & CHURN=$!"
Enter
Type "clear"
Enter
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Show
Sleep 500ms

Type "g"
Sleep 600ms
Type "d"
Sleep 5s

# Move across the tiles, then open Ops/sec: the raw INFO `stats` section behind it.
Right
Sleep 800ms
Down
Sleep 800ms
Left
Sleep 800ms
Enter
Sleep 4s
Escape
Sleep 1500ms

Hide
Type "q"
Sleep 400ms
Type "kill $CHURN"
Enter
Sleep 300ms
