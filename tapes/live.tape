# live: the open key follows the server, a TTL counts down, and the key is deleted.
#
# Shows the header's live status as another client changes `demo:live` (fields flash, with no
# keypress), the TTL ticking down, and then the key deleted from another client: the Viewer keeps
# the last value, badged `deleted`, instead of going blank.
# Embedded in: viewing/live-updates.
#
# The writer is a hidden background loop using the demo container's own redis-cli, killed (and
# the container removed) by render.sh. Keys: `t` tree/flat, `/` filter, `⏎` apply, `→` open, `q` quit.
#
# poster: 9

Source "_settings.tape"
Output ../site/public/demos/live.mp4

Require redis-pane

Hide
Type "R='docker exec redis-pane-demo redis-cli'"
Enter
Type "(sleep 9; for i in 1 2 3 4; do $R HSET demo:live status running hits $((i*7)) >/dev/null; sleep 1.6; done; sleep 1.5; $R DEL demo:live >/dev/null) & W=$!"
Enter
Type "clear"
Enter
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Type "t"
Sleep 400ms
Type "/demo:live"
Enter
Sleep 800ms
Show
Sleep 400ms

Right
Sleep 15s

Hide
Type "q"
Sleep 400ms
Type "kill $W 2>/dev/null"
Enter
Sleep 300ms
