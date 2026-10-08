# redis-pane

`redis-pane` is a terminal UI for browsing and editing Redis. It is one small binary that runs
in the terminal you are already in, including an SSH session on a bastion host.

![Browsing a keyspace in redis-pane: filtering to a key, then watching it update on screen the moment another client changes it, with no keypress or refresh](images/demo.gif)

*Filter to a key, open it, and watch it update the moment another client changes it. There is
no refresh button.*

## Who it is for

You use Redis, and `redis-cli` can't show you what is in your keyspace. You want to see keys
as a tree, open one, read it, and change it, all from the keyboard. You do this on machines
where a desktop GUI is not an option.

## What it does

- **Browses a keyspace of any size.** Keys stream in with `SCAN` (never `KEYS`) and the list
  stays usable while they arrive. See [The key list](browsing/key-list.md).
- **Shows every type.** Strings, Hashes, Lists, Sets, Sorted sets, Streams, JSON and binary
  values each get a purpose-built view. See [Types](viewing/types.md).
- **Updates the open key live.** When another client changes it, the screen changes with no
  keypress. See [Live updates](viewing/live-updates.md).
- **Edits in place.** Change a value, a TTL or a name. Every change previews the exact command
  before it runs. See [Editing values](editing/values.md).
- **Knows where it is pointed.** Each target has an Environment (`local`, `staging`, `prod` or
  `unknown`) shown in the title bar, and `prod` and `unknown` start in Read-only Mode. See
  [Environments](safety/environments.md).
- **Watches the server.** A Dashboard, a live Monitor, Pub/Sub and the Slowlog are each a
  keystroke away. See [Server views](server/dashboard.md).
- **Works with Sentinel and Cluster.** See [Sentinel](connecting/sentinel.md) and
  [Cluster](connecting/cluster.md).

It needs Redis 6.0 or newer. Valkey works too.

## How it compares

| You want to… | `redis-cli` | `redis-pane` |
| --- | --- | --- |
| See keys as a tree, with type, size and TTL | Names only (`--scan`) | Yes |
| Open a key and read it by type | Run the type's read command yourself | Yes |
| Watch a key change as it happens | No | Yes, pushed by the server |
| Run any Redis command | Yes | No: [no console](limits.md) |
| Run in a script or a pipeline | Yes | No |
| Work over SSH, with the clipboard | Not applicable | Yes, via [OSC 52](viewing/copying.md) |
| Edit with a preview of the exact command | No | Yes |
| Be told which Environment you are pointed at | No | Always, in the title bar |

`redis-pane` doesn't replace `redis-cli` for scripts or for commands it has no key for. Use
both.

## Where to go next

- New here? [Install it](getting-started/installation.md), then take the
  [quick start](getting-started/quick-start.md).
- Connecting to something real? Start with
  [how the target is chosen](connecting/target.md).
- Looking for a key? The [keybindings](reference/keybindings.md) page lists every one.
