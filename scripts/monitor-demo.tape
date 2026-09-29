# monitor-demo.tape — records a short terminal demo of the Monitor view
# (`g m`, R6.1) with VHS (https://github.com/charmbracelet/vhs —
# `brew install vhs`).
#
# Shows: jumping to Monitor with the `g m` chord, the persistent cost banner,
# live commands streaming in from another client, pausing (lines are counted,
# not buffered), resuming, and narrowing the tail with `/` — then `Esc`
# closing the feed on the way back to the key browser.
#
# Render with:
#   vhs scripts/monitor-demo.tape
# which produces monitor-demo.gif in the repo root.
#
# A streaming tail redraws most of the screen on every command, which GIF
# compresses badly (the raw render is ~4MB, ~2.5MB after this). Shrink
# it with a small shared palette and diff rectangles before committing it:
#   ffmpeg -i monitor-demo.gif -vf "split[a][b];[a]palettegen=max_colors=32:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" monitor-demo.min.gif
#   mv monitor-demo.min.gif monitor-demo.gif
# Stop any other churn.py first: it is the only traffic this tape expects,
# and a second writer doubles both the noise and the file size.
#
# Prereqs (run once, before recording — see scripts/README.md):
#   ./scripts/redis-up.sh
#   python3 scripts/fixtures.py --flush --seed 7
#   cargo build --release -p redis-pane
#   export PATH="$PWD/target/release:$PATH"
#
# Afterwards: ./scripts/redis-up.sh down
#
# The recording targets --url redis://127.0.0.1:6379 explicitly (not plain
# `redis-pane`) for the same reason demo.tape does: connection resolution is
# flags -> default Profile -> environment -> 127.0.0.1:6379 (CLAUDE.md), and
# a bare `redis-pane` would prefer a configured default Profile over this.
# Loopback also makes the Environment `local`, so `g m` opens straight away —
# on `prod` or `unknown` it would ask first.
#
# Keybindings below match DESIGN §4 and §6.7 as of this writing. Keybindings
# are data (CLAUDE.md) and user-overridable, so if a step doesn't do what its
# comment says, check `?` in a live session and adjust the tape rather than
# assuming the recording is stale.

Require redis-pane
Require python3

Output monitor-demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 1200
Set Height 720
Set Theme "Dracula"
Set Padding 20
Set TypingSpeed 50ms
Set Framerate 10

# Start a background writer so there is live traffic for MONITOR to show,
# without ever showing this setup command in the recording. It stops itself
# after --duration seconds, so nothing is left running after the tape ends.
Hide
Type "python3 scripts/churn.py --rate 3 --duration 45 --no-delete -q > /tmp/redis-pane-monitor-demo-churn.log 2>&1 & disown"
Enter
Sleep 500ms
Type "clear"
Enter
Show

Type "redis-pane --url redis://127.0.0.1:6379"
Enter
Sleep 2s

# `g m` — a view chord. `g` alone waits for its second key, with no timeout.
Type "g"
Sleep 900ms
Type "m"
Sleep 5s

# Pause: the socket stays open, but new lines are counted rather than kept —
# watch the skipped count climb while the tail holds still.
Type "p"
Sleep 3s

# Resume: no backfill of what was skipped, the tail just carries on.
Type "p"
Sleep 3s

# Narrow the tail to one command. The filter only changes what is shown,
# never what is buffered.
Type "/"
Sleep 300ms
Type "hset"
Sleep 800ms
Enter
Sleep 4s

# Clear the filter again.
Type "/"
Sleep 300ms
Backspace 4
Sleep 300ms
Enter
Sleep 2500ms

# Leaving the view closes the feed — MONITOR never keeps running out of sight.
Escape
Sleep 1500ms

Type "q"
Sleep 500ms
