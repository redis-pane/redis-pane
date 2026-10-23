# social: the whole pitch in one captioned take, for posts (X, Bluesky, Mastodon; muted autoplay).
#
# Six scenes, about 45 s, a caption burned in under each by render.sh from tapes/social.captions:
#   1 Live        a hash another client writes every 0.7 s; hits and status change with no keypress
#   2 Tree        fold and unfold groups, counts on every group
#   3 Search      `/` substring narrows as you type, then a glob
#   4 Safe edit   edit JSON, stage, read the exact command and diff, confirm
#   5 Dashboard   `g d`, Ops/sec moving under load
#   6 Monitor     `g m`, commands streaming
#
# Not embedded on any page; it lives at https://redis-pane.github.io/redis-pane/demos/social.mp4
# for downloading and posting. Longer than the site's 8 to 20 s clips, so it has its own 3 MB
# budget (tapes/README.md).
#
# The writer is tapes/_social_writer.py (hits/status on demo:live, then mixed traffic from
# --traffic-after, which is scene 5). It ends by itself (--duration) and is killed at the end.
#
# Render with ./tapes/render.sh social (it owns the Redis on :6390, the seed, the XDG dirs and
# the captions). Keys are from site/content/docs/reference/keybindings.md: `t` tree/flat,
# `/` filter, `⏎` apply, `Esc` clear & exit, `←`/`→` collapse/expand, `e` edit, `⌃S` stage,
# `y` confirm, `g d` dashboard, `g m` monitor, `q` quit.
#
# poster: 5

Source "_settings.tape"
Output ../site/public/demos/social.mp4

Require redis-pane
Require python3

Hide
Type "python3 _social_writer.py --port 6390 --start-after 6.5 --traffic-after 38 --duration 90 >/dev/null 2>&1 & WRITER=$!"
Enter
Type "clear"
Enter
Type "redis-pane --url redis://127.0.0.1:6390"
Enter
Sleep 2500ms

# ---- 1 Live: the recording opens on the hash, which another client is writing -----------------
# Flat list, filter to the key and open it off camera, so every second of scene 1 is the payoff.
Type "t"
Sleep 400ms
Type "/demo:live"
Sleep 300ms
Enter
Sleep 400ms
Right
Sleep 600ms
Show
Sleep 7s

# ---- 2 Tree: clear the filter and go back to the tree off camera (a cut, no flicker) --------
Hide
Escape
Sleep 300ms
Type "/"
Sleep 200ms
Escape
Sleep 300ms
Type "t"
Sleep 600ms
Show
Sleep 800ms
Left
Sleep 900ms
Down
Sleep 400ms
Left
Sleep 900ms
Down
Sleep 400ms
Left
Sleep 900ms
Right
Sleep 1200ms
Right
Sleep 1s
Down 2
Sleep 800ms

# ---- 3 Search: substring, then glob ---------------------------------------------------------
Type "/"
Sleep 300ms
Type "session"
Sleep 1200ms
Enter
Sleep 700ms
Type "/"
Sleep 200ms
Backspace 7
Type "user:*:session"
Sleep 1200ms
Enter
Sleep 1500ms

# ---- 4 Safe edit: demo:config ---------------------------------------------------------------
Type "/"
Sleep 200ms
Backspace 14
Type "demo:config"
Sleep 300ms
Enter
Sleep 600ms
Right
Sleep 500ms
Enter
Sleep 900ms
Type "e"
Sleep 600ms
Down 4
Right 13
Delete 2
Type "13"
Sleep 500ms
Ctrl+S
Sleep 3s
Type "y"
Sleep 1s

# ---- 5 Dashboard ----------------------------------------------------------------------------
Type "g"
Sleep 300ms
Type "d"
Sleep 5700ms

# ---- 6 Monitor ------------------------------------------------------------------------------
Escape
Sleep 300ms
Type "g"
Sleep 300ms
Type "m"
Sleep 3s
Type "p"
Sleep 2s

# Quit and stop the writer off camera.
Hide
Type "q"
Sleep 500ms
Type "kill $WRITER"
Enter
Sleep 300ms
