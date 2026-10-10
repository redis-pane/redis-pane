# pubsub: subscribe to a channel and to a pattern, and watch the messages arrive.
#
# Shows `g p`, `a` adding a channel (`news`) and then a pattern (`user:*`, whose chip carries a
# trailing `*`), messages arriving from a hidden publisher (tapes/_publisher.sh), a JSON payload
# pretty-printed in the detail strip, and `Esc` leaving, which closes the subscription connection.
# Embedded in: server/pubsub.
#
# Keys are from site/content/docs/reference/keybindings.md: `g p` pub/sub, `a` add, `⏎` subscribe,
# `p` pause, `Esc` back, `q` quit.
#
# poster: 9

Source "_settings.tape"
Output ../site/public/demos/pubsub.mp4

Require redis-pane

Hide
Type "(./_publisher.sh 8 &) ; clear"
Enter
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms
Show
Sleep 500ms

Type "g"
Sleep 500ms
Type "p"
Sleep 1500ms

# With no subscriptions yet, the view opens the add prompt itself.
Type "news"
Sleep 500ms
Enter
Sleep 1500ms

# A name with a wildcard is a pattern (PSUBSCRIBE).
Type "a"
Sleep 500ms
Type "user:*"
Sleep 500ms
Enter
Sleep 6s

# Pause: messages are counted and dropped, then the tail carries on.
Type "p"
Sleep 2s
Type "p"
Sleep 1500ms

Hide
Escape
Sleep 500ms
Type "q"
Sleep 400ms
