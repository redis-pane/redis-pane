# safety: a prod Profile opens read-only; the preview comes first; y is refused; ⌃R lifts it.
#
# Shows the title bar's `READ-ONLY environment`, `d` still showing the exact command, the refusal
# at `y` (with the reason), the help overlay with the keys Read-only Mode would refuse dimmed, and
# `⌃R` turning it off, after which `d` stages the same command, which we discard with `Esc`.
# Embedded in: safety/read-only, safety/previews.
#
# The `prod` Profile is written by render.sh into the temp XDG config (chmod 600), pointing at the
# demo Redis on :6390; it is selected by name, so no real config can be read.
# Keys are from site/content/docs/reference/keybindings.md: `→` open, `d` delete, `y` confirm,
# `Esc` discard, `?` help, `⌃R` read-only, `q` quit.
#
# poster: 7

Source "_settings.tape"
Output ../site/public/demos/safety.mp4

Require redis-pane

Hide
Type "redis-pane prod"
Enter
Sleep 3s
Type "t"
Sleep 500ms
Type "/demo:safe"
Enter
Sleep 800ms
Show
Sleep 1800ms

# The preview shows the command first.
Type "d"
Sleep 2500ms

# Read-only Mode refuses at `y`, not before.
Type "y"
Sleep 2500ms

# Help dims the keys that Read-only Mode would refuse, with the reason.
Type "?"
Sleep 3500ms
Escape
Sleep 800ms

# ⌃R lifts it (the reason here is the environment, not a replica). Same key, same command.
Ctrl+R
Sleep 2s
Type "d"
Sleep 2500ms
Escape
Sleep 1s

Hide
Type "q"
Sleep 400ms
