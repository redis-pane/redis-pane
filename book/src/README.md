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

| | redis-pane | `redis-cli` | RedisInsight |
| --- | :---: | :---: | :---: |
| Runs in a terminal and over SSH | ✓ | ✓ | ✗ [^1] |
| Single binary | ✓ (2.7 MB download [^2]) | ✓ | ✗ [^3] |
| The open key updates live, by push, with no refresh | ✓ | ✗ | partial [^4] |
| A million keys browsable | ✓ (measured [^5]) | partial [^6] | not measured [^7] |
| The exact command is shown before every write | ✓ | ✗ [^8] | not documented [^9] |
| Writes are locked by Environment | ✓ | ✗ [^8] | not documented [^9] |
| Cluster | ✓ | ✓ [^10] | ✓ [^11] |
| Sentinel | ✓ | ✗ [^12] | ✓ [^11] |

[^1]: RedisInsight describes itself as a "desktop GUI client" built on Electron, also shipped as a Docker image. It has no terminal interface.
[^2]: The `redis-pane-aarch64-apple-darwin.tar.xz` asset of release `0.1.0-beta.4` is 2.7 MB. The four platform downloads range from 2.7 MB to 5.5 MB (the Windows zip).
[^3]: An Electron desktop application, or a Docker image, per its README.
[^4]: Its documentation describes a configurable automatic refresh rate for Streams and for the Slow Log. It documents no push update for an open key's value.
[^5]: `crates/core/tests/perf.rs` drives 1,000,000 keys in real `SCAN` order. No single update, which includes any slice of a rebuild, takes longer than the 16 ms frame budget.
[^6]: `redis-cli --scan` streams the names of every key, with `--pattern` to filter. It doesn't browse, show types or open values.
[^7]: We have not measured other tools and make no claim about them.
[^8]: `redis-cli` runs what you type. Nothing previews it and nothing locks it.
[^9]: We found no mention of a command preview, a read-only mode or an Environment lock in its README or documentation. We have not tested it, so this is "not documented", not "absent".
[^10]: `redis-cli -c` follows `-MOVED` and `-ASK` redirections, and `--cluster` runs the cluster manager commands.
[^11]: Its documentation says you can add "any Redis database running anywhere (including Redis Open Source cluster or sentinel)".
[^12]: `redis-cli --help` has no Sentinel option. A Sentinel node can be queried as an ordinary server.

`redis-pane` doesn't replace `redis-cli` for scripts or for commands it has no key for. Use
both.

## Where to go next

- New here? [Install it](getting-started/installation.md), then take the
  [quick start](getting-started/quick-start.md).
- Connecting to something real? Start with
  [how the target is chosen](connecting/target.md).
- Looking for a key? The [keybindings](reference/keybindings.md) page lists every one.
