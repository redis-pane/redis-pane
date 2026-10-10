---
title: "Quick start"
---

## Point it at a Redis

With no arguments, `redis-pane` connects to `127.0.0.1:6379`:

```bash
redis-pane
```

For anything else, give it a URL:

```bash
redis-pane --url redis://localhost:6379
```

Or use a Profile you've saved in the [config file](../connecting/profiles.md):

```bash
redis-pane staging
```

If you don't have a Redis handy, `brew install redis && brew services start redis` gets you one,
or start the disposable one in `scripts/` (see [Contributing](../contributing.md)).

The title bar always shows the target, its Environment and where the target came from. Read it
before you press anything. See [How the target is chosen](../connecting/target.md).

## Look around

- `↑` `↓` or `j` `k` move through the key list.
- `→` or `l` opens a key, or expands a tree group. `←` or `h` collapses a group or jumps to its
  parent.
- `/` filters the list. `Esc` clears the filter.
- `t` switches between the tree and a flat list. `s` cycles the sort order.
- `Enter` puts a cursor inside the open value. `Esc` takes you back to the list.
- `Tab` moves focus between the two panes.

![Folding a tree group with Left and Right, filtering down to one key, and moving a cursor through its value with Enter and the arrow keys](/images/navigation-demo.gif)

## Ask for help

`?` (or `F1`, which also works while you are typing) opens help for exactly where you are. It
lists the keys that work here, and dims anything Read-only Mode would refuse, with the reason.

## Try a change

On a `local` target, open a String key and press `e`. Edit the text, then press `⌃S`. You'll
see the exact command that will run. `y` runs it, and `Esc` throws it away. See
[Editing values](../editing/values.md).

## Quit

`q` or `⌃C`.

## Next

- [How the target is chosen](../connecting/target.md)
- [Environments](../safety/environments.md)
- [Keybindings](../reference/keybindings.md)
