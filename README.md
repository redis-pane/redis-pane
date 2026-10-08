# redis-pane

A terminal UI for browsing Redis — the keyspace viewer `redis-cli` never gave you.

`redis-cli` is great for running commands, but it can't show you what's actually in your
keyspace. RedisInsight can, but it's a heavy Electron app that struggles with large keyspaces and
doesn't work over SSH. `redis-pane` is a single small binary that runs right in your terminal,
keyboard-driven, and stays fast even with huge keyspaces.

![Browsing a keyspace in redis-pane: filtering to a key, then watching it update on screen the moment another client changes it, with no keypress or refresh](demo.gif)

*Filter to a key, open it, and watch it update live the moment it changes on the server — no
refresh needed.*

## Status

This is a beta (`0.1.0-beta.4`). It handles browsing a keyspace, viewing every Redis type, and live updates
when a value changes on the server. You can edit every core type in place — a String value, Hash
fields, List elements, Set members, Sorted set scores — change a key's TTL, and delete a key (`e`
to edit, `a` to add, `d` to delete, `t` for the TTL, `y` to confirm). Every change previews the
exact command before it runs, and Read-only Mode refuses it where it should. Editing happens
inline, so a value that reads as JSON stays visible while you type.

It can also watch the server itself: a live Monitor tail, the Slowlog, and a Dashboard of the
server's own vitals, each a keystroke away. It ships dark, light and high-contrast themes, an ASCII
fallback for terminals without Unicode, and restores your pane split, filter, sort and selected key
when you relaunch against the same server.

It works against a Redis Cluster too: point it at a `redis-cluster://` URL or any one node. The
key list covers every primary, an open key stays live on whichever node owns it (through failovers
and slot moves), edits go to the owner, and every server view is cluster-aware — a Dashboard with
a node table, the Slowlog and Monitor merged across nodes, and sharded Pub/Sub.

It stays responsive at a million keys arriving in real `SCAN` order: no keystroke or scan page
waits on a rebuild. A large rebuild (toggling the tree, changing the sort, rebuilding the filter)
runs in slices between frames while the old list stays usable under a `rebuilding N%` readout,
and the new list arrives in about a tenth of a second — a tree toggle ~0.1 s, a sort change
~0.1 s, a collapse or a filter rebuild a few hundredths.

Keys can be renamed (`R`), duplicated (`D`, Redis `COPY`) and deleted in bulk (`Space` marks,
`d` deletes the marked set — on `prod` or `unknown` you type the count), and a Hash field or a Set/Sorted-set
member can be renamed in place (`R` in the value pane). Nothing ever overwrites an existing name.

Known limits: no move-across-database, and no opening a value in your own `$EDITOR` yet.

Requires Redis 6.0 or newer (Valkey works too).

## Install

Download the binary for your OS from the [Releases page](https://github.com/vinodsantharam/redis-pane/releases) — macOS, Linux, and Windows are all covered. Or run the install script:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/vinodsantharam/redis-pane/releases/download/v0.1.0-beta.4/redis-pane-installer.sh | sh
```

On Windows, from PowerShell:

```powershell
irm https://github.com/vinodsantharam/redis-pane/releases/download/v0.1.0-beta.4/redis-pane-installer.ps1 | iex
```

These beta builds are unsigned, so your OS may flag them on first run — on macOS, right-click the
binary → Open → confirm; on Windows, click "More info" → "Run anyway" in the SmartScreen prompt.

Prefer to build it yourself? You'll need [Rust](https://rustup.rs):

```bash
git clone https://github.com/vinodsantharam/redis-pane.git
cd redis-pane
cargo build --release
```

The binary lands at `./target/release/redis-pane`.

## Using it

Point it at a Redis server with a URL:

```bash
redis-pane --url redis://localhost:6379
```

Or with individual flags:

```bash
redis-pane --host your-host --port 6379 --user default --password your-password --tls
```

With no arguments, it connects to `127.0.0.1:6379`. For anything you connect to often, save it as
a Profile in `~/.config/redis-pane/config.json` — see [Profiles and the config file](book/src/connecting/profiles.md) for a full
walkthrough, and [Passwords and secrets](book/src/connecting/passwords.md) for keeping passwords out
of your shell history.

Once you're in:

- `↑↓` or `j`/`k` to move, `→` or `l` to open a key (or expand/descend a tree group), `←` or `h`
  to collapse a group or jump to its parent
- `Enter` to open the selected key and move a cursor inside its value, `Esc` to leave it
- `/` to filter, `Esc` to clear
- `t` to toggle tree/flat view, `s` to cycle sort order
- `c` to copy the key name or the value, whichever pane is focused; `C` to copy a `redis-cli`
  command for the open key (locally this uses the system clipboard directly; over SSH it relies
  on OSC 52 — see [Copying](book/src/viewing/copying.md) if a paste comes back empty)
- `?` (or `F1`, which works even while typing) for help with what the keys do right where you are

![Folding a tree group with Left/Right, filtering down to one key, and moving a real cursor through its value with Enter and the arrow keys](navigation-demo.gif)

*`←`/`→` fold and step through the tree; `Enter` drops a cursor into the open value so you can move
through a long list without the mouse, and `Esc` takes you back to the key list.*

Beyond the keyspace, four views show what the server itself is doing:

- `g m` opens **Monitor**, a live tail of every command the server runs. `p` pauses it, `/`
  filters it, and leaving the view shuts the feed down so it never keeps running out of sight
- `g p` opens **Pub/Sub**: add a channel or pattern and watch what's published to it, on its own
  connection so it never competes with your reads. `Tab` in the add form makes it a **sharded**
  subscription (Redis 7+, `SSUBSCRIBE`, the kind a Cluster delivers only from the channel's own
  node). `d` unsubscribes a chip, `p` pauses the tail, and leaving the view closes the connection —
  nothing stays subscribed behind you
- `g s` opens the **Slowlog**, the server's own list of commands that ran too long, sortable by
  time or duration. On a Cluster it merges every node's log with a NODE column, and `d` resets
  them all
- `g d` opens the **Dashboard**: memory against `maxmemory`, hit ratio, an ops/sec sparkline,
  clients, replication lag and eviction counters, polling `INFO` every couple of seconds. Anything
  alarming is colored, and `Enter` on a tile expands the raw `INFO` section behind it. On a
  Cluster it adds a `cluster_state` health line, tiles summed across the nodes and a node table;
  `j`/`k` move along it and `Enter` opens one node's own tiles (`Esc` goes back)

![The Monitor view: a live tail of commands streaming in under a warning banner, paused while the skipped count climbs, then filtered down to HSET commands](monitor-demo.gif)

*`g m` and every command the server runs streams in as it happens. Pause it without losing your
place, filter it down to what you're chasing, and the banner never lets you forget that MONITOR
costs the server while it's open.*

## Trying it out

See [TESTING.md](TESTING.md) for what to try in this beta and how to report what you find, and the
[user guide source](book/src/README.md) for everything else.

## Contributing

Working on the code? [CLAUDE.md](CLAUDE.md) has the architecture notes and conventions, and
[CONTEXT.md](CONTEXT.md) is a short glossary of terms used throughout the project. Deeper design
docs live under `docs/` if you want the full history behind a decision.

## Not planning to build

Not a server manager, not an alerting or paging tool, and not a metrics store — the Dashboard reads
`INFO` live and keeps nothing beyond an in-session sparkline, it does not retain history across a
restart or notify anyone — and not a replacement for `redis-cli` in scripts. No multi-server
workspace, no plugins, no export/import — one connection at a time, kept simple.
