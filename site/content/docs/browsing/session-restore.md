---
title: "Session restore"
---

Quit and relaunch against the same server and you're back where you left off. Five things are
remembered:

- the pane split (how far you moved the divider with `⌃←` and `⌃→`)
- tree or flat
- the sort order
- the filter
- the Selected key

If the key has since gone, the cursor starts at the top.

It's remembered **per target**, for example `cache-01:6379/0`, so `staging`'s filter never leaks
into a `prod` session. There is no "restored" banner. It just happens.

## Where it's kept

In `$XDG_STATE_HOME/redis-pane/state.json`, or `~/.local/state/redis-pane/state.json` if that
variable isn't set. It is separate from your [config file](../connecting/profiles.md), which
`redis-pane` never writes. The file holds target names without passwords, and keeps the 32 most
recent targets.

If the file is corrupt or comes from an incompatible version, you get a notification saying so
and the session starts clean. Delete the file to forget everything.
