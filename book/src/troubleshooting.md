# Troubleshooting and FAQ

## The binary won't open

The beta builds aren't signed. On macOS, right-click the binary, choose Open and confirm, or run
`xattr -d com.apple.quarantine ./redis-pane`. On Windows, click "More info" then "Run anyway" in
the SmartScreen prompt. See [Installation](getting-started/installation.md).

## It connected to the wrong place

Read the title bar. It shows the target, the Environment and the Source. `from default` means
nothing else applied and you're on `127.0.0.1:6379`. Check with `redis-pane --print-target`. A
`--profile` name that isn't in your config file is an error (`no Profile named "..."`, exit code
`3`), not a fallback. See [How the target is chosen](connecting/target.md).

## The config file is refused

`redis-pane` won't start, and prints the reason:

- **`refusing to read a config file with mode 0644`**: the file is readable by others. Run the
  `chmod 600` it prints.
- **`unknown field` with a line and column**: a field name is misspelled or isn't supported. The
  message lists the fields that are. Fix the name.
- **`defaultProfile names "x", which is not defined`**: the name must be a key of `profiles`.
- **A theme error**: `theme` must name a built-in or one of your `themes`.

Exit code `3` means a config problem. See [Configuration](reference/configuration.md).

## It can't connect

The message names the target, its Source and the reason. Exit code `2` is a connection that
failed or was refused. Common causes:

- A password is missing or wrong. If it comes from `passwordEnv`, check the variable is set in
  the shell you launched from. If it comes from `passwordCommand`, run the command by hand: only
  its exit status is reported.
- The server needs TLS. Add `--tls`, or use `rediss://`.
- A Sentinel URL without `sentinelServiceName`. See [Sentinel](connecting/sentinel.md).

Check a Profile without opening the UI: `redis-pane --profile NAME --probe`.

## "Redis older than 6.0"

`redis-pane` needs Redis 6.0 or newer, and the RESP3 protocol. On an older server it exits with
code `4` and names the version it found. Valkey works.

Some commands need more: duplicating a key (`D`) needs Redis 6.2, and a sharded subscription
needs Redis 7. These are dimmed in help with the reason.

## The header says `○ manual`, not `● live`

The server refused to track the open key, or the tracking was lost. `r` re-reads by hand, and the
header shows how old the value is. Run `redis-pane --probe` and look at the `liveness:` line. If
it says `CLIENT TRACKING refused`, the server (often a managed service) doesn't allow the
`CLIENT` command. See [Live updates](viewing/live-updates.md).

If a key never goes `● live` and `--probe` says tracking is accepted, report it.

## A paste over SSH is empty

Copying over SSH uses OSC 52, which depends on your terminal. Check that your terminal allows
programs to write to the clipboard. The setting is often called "allow clipboard access" or
"OSC 52". Known to work: iTerm2, kitty, WezTerm, Alacritty, foot and Windows Terminal. Some
terminals ignore it entirely.

In tmux, add `set -g set-clipboard on` to `~/.tmux.conf`. `redis-pane` can't tell "the terminal
ignored it" from "it worked", because OSC 52 has no reply. See [Copying](viewing/copying.md).

## Boxes or question marks instead of symbols

Run `redis-pane --ascii`. To make it permanent, add `"ascii": true` to the config file. See
[ASCII and colour](appearance/ascii-and-colour.md).

## Colours look wrong

Check `COLORTERM` and `TERM`. Set `NO_COLOR=1` for monochrome, or try `--theme light` or
`--theme high-contrast`, which paint their own background. See
[Themes](appearance/themes.md).

## A key I just created isn't in the list

The list is the result of a scan. Press `r` in the key list to scan again.

## A key that I know is there is missing

Look at the status bar. The scan may still be running, or have stopped at the
[2,000,000 key limit](browsing/large-keyspaces.md), or a filter may be hiding it.

## A Cluster node is gone

A node that stops answering doesn't stop the session. The server views carry on with the other
nodes and name the one that failed, with the command and the reason. In Monitor, a line under the
status names the stopped feed. The key list and the open key follow failovers and slot moves.

If a topology change interrupts a scan, the list says so. You keep the keys loaded so far, and
`r` rescans.

## Something else

Open an issue on the [GitHub repository](https://github.com/redis-pane/redis-pane/issues).
Say what you were doing, what you expected, what happened, and what you were connected to (Redis
version, and local, Upstash, Redis Cloud or something else).
