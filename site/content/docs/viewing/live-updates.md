---
title: "Live updates"
---

The open key updates itself. When another client changes it, the new value appears with no
keypress, and the fields that changed are briefly highlighted.

There is no refresh button, because there is nothing to refresh. `redis-pane` doesn't keep a
copy of the value. It asks the server to tell it when the open key changes (Redis calls this
`CLIENT TRACKING`), and reads the key again when the server says so. If another client changes
the key, you see it, and you never have to wonder whether the screen is stale.

Only the open key is watched. The key list isn't live. To pick up new keys, press `r` in the
list to rescan.

## What the header tells you

The header always says what is going on, so you can tell "nothing changed" from "the update
didn't arrive".

| Header | Means |
| --- | --- |
| `● live` | The value is current, and the server will tell us if it changes. |
| `● live · updated now` | An update just landed. |
| `● live · unchanged` | A read found no change. |
| `● live · changed 2s ago  r` | It changed while you were scrolled into the value. Nothing moved under your cursor. Press `r` to bring it up to date. |
| `✎ editing · changed · held` | It changed while you were editing. Your unsaved text is never touched. |
| `✕ deleted 3s ago` | The key is gone from the server. |
| `○ manual · read 14s ago  r` | Live updates aren't available. `r` re-reads. |

The "updated now" and "unchanged" notes fade after a couple of seconds. They report what the last
read found, then stop.

## `r` is a Refetch

`r` in the value pane re-reads the open key from the server: type, size, TTL and value. It's
never a global refresh, and in the key list, `r` means a rescan instead. The hint bar shows
which, as `r refetch` or `r rescan`.

## Degraded to manual

If the server won't do live updates, the header says `○ manual`, shows how long ago the value was
read, and `r` does the work. This happens on some managed Redis services that disable the
`CLIENT` command. `redis-pane --probe` tells you in advance: it prints
`CLIENT TRACKING refused — degrading to manual`.

It's never silent. If the header doesn't say `● live`, don't treat the value as current.

## After a lost connection

If the connection drops, `redis-pane` stays usable, keeps the last value on screen and reconnects
in the background with a visible countdown. Press `r` to retry now. After it reconnects, it
re-arms live updates **before** the header says `● live` again. A reconnected session that isn't
yet watching your key doesn't claim to be.

## A key that disappears

If the open key is deleted, expires or is evicted, the Viewer keeps the last value it read,
badged `✕ deleted 3s ago`, and turns off anything that would write. During an incident the
question is usually *what was in it?*, and that answer would otherwise be gone.

## When the open key isn't the selected key

The value pane keeps showing the Open key while you move the cursor elsewhere. It says so with
`⊘ not the selected key`, or `⊘ not in the list` if a filter or a collapsed group hides it.
The value is still live. See [The key list](../browsing/key-list.mdx).
