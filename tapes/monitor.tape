# monitor: `g m`, a live tail of every command, pause, and filter.
#
# Shows the Monitor view's persistent cost banner, commands streaming in from another client,
# pausing (lines are counted, not buffered), resuming, and narrowing the tail with `/`.
# Embedded in: server/monitor, the README (GIF).
#
# Keys are from site/content/docs/reference/keybindings.md: `g m` monitor, `p` pause/resume,
# `/` filter, `⏎` apply, `Esc` back, `q` quit. Loopback makes the Environment `local`, so `g m`
# opens straight away (on `prod` or `unknown` it asks first).
#
# A streaming tail redraws most of the screen on every command, which compresses badly, so this
# one runs at a lower frame rate and the webm/mp4 CRF in render.sh is what keeps it small.
# Churn is the only traffic: a second writer would double both the noise and the file size.
#
# poster: 8
# gif: yes

Source "_settings.tape"
Output ../site/public/demos/monitor.mp4
Set Framerate 10

Require redis-pane
Require python3

# Hidden writer, so there is live traffic for MONITOR. It stops itself after --duration seconds
# and is killed at the end of the tape.
Hide
Type "python3 ../scripts/churn.py --port 6390 --rate 4 --no-delete --duration 60 -q >/dev/null 2>&1 & CHURN=$!"
Enter
Type "clear"
Enter
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Show
Sleep 500ms

# `g m` is a view chord: `g` alone waits for its second key.
Type "g"
Sleep 700ms
Type "m"
Sleep 4s

# Pause: the socket stays open, but new lines are counted rather than kept.
Type "p"
Sleep 2500ms

# Resume: no backfill of what was skipped, the tail just carries on.
Type "p"
Sleep 2500ms

# Narrow the tail to one command. The filter changes what is shown, never what is buffered.
Type "/"
Sleep 300ms
Type "hset"
Sleep 600ms
Enter
Sleep 3s

# `Esc` leaves the view, which closes the feed: MONITOR never keeps running out of sight.
Escape
Sleep 1500ms

Hide
Type "q"
Sleep 400ms
Type "kill $CHURN"
Enter
Sleep 300ms
