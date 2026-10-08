# Copying

| Key | Where | Copies |
| --- | --- | --- |
| `c` | Key list | The Selected key's name. |
| `c` | Value pane | The value, as the Viewer shows it: one row per line, cells separated by tabs. A String is copied exactly as it is. |
| `C` | Key list or value pane | A `redis-cli` command that reads this key from this server, for example `redis-cli -h cache-01 -p 6379 -n 0 HGETALL user:8812:session`. |

`C` picks the read command that suits the type, so the copied command works. Passwords are never
included. On a [Cluster](../connecting/cluster.md) the command is `redis-cli -c -h ... -p ...`
with no `-n`.

The server views have their own `c`: it copies a Monitor command, a Pub/Sub payload, a Slowlog
command or the raw `INFO` section.

## How it reaches your clipboard

The method is chosen automatically.

**On macOS, outside SSH:** `redis-pane` calls `pbcopy`.

**Over SSH, and on any other system:** it uses **OSC 52**, a terminal escape sequence that hands
the text to the terminal on your own machine. This is what makes copying work from a bastion host:
a native clipboard call there would fill the remote machine's clipboard, which is nobody's.
`redis-pane` decides it's in an SSH session when `SSH_TTY`, `SSH_CONNECTION` or `SSH_CLIENT` is
set.

OSC 52 depends on your terminal. It's known to work in iTerm2, kitty, WezTerm, Alacritty, foot,
Windows Terminal and tmux (with `set -g set-clipboard on`). Some terminals ignore it, and there
is no reply, so `redis-pane` can't tell "the terminal ignored it" from "it worked".

OSC 52 text is capped at about 74,000 bytes, because terminals drop longer sequences whole. A
longer copy is cut, and the notice says so.

If a paste comes back empty, see [Troubleshooting](../troubleshooting.md#a-paste-over-ssh-is-empty).
