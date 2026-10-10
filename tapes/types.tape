# types: one key of each type, and what its viewer shows.
#
# Walks a binary value (hex and ASCII), Hash, JSON string, List, Set, Stream and Sorted set, all in the same frame: header (key, type, size, TTL, live status), body, footer.
# Embedded in: viewing/types.
#
# Keys are from site/content/docs/reference/keybindings.md: `t` tree/flat, `/` filter, `⏎` apply,
# `→` open, `Tab` focus, `↓` move, `q` quit.
#
# poster: 6

Source "_settings.tape"
Output ../site/public/demos/types.mp4

Require redis-pane

Hide
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 3s
Type "t"
Sleep 500ms
Type "/demo:t:"
Enter
Sleep 1s
Show
Sleep 500ms

# demo:t:binary
Right
Sleep 1500ms
Tab
Down
# demo:t:hash
Right
Sleep 1500ms
Tab
Down
# demo:t:json
Right
Sleep 1500ms
Tab
Down
# demo:t:list
Right
Sleep 1500ms
Tab
Down
# demo:t:set
Right
Sleep 1500ms
Tab
Down
# demo:t:stream
Right
Sleep 1500ms
Tab
Down
# demo:t:zset
Right
Sleep 1500ms

Hide
Type "q"
Sleep 400ms
