# Slowlog

`g s` opens the Slowlog: the server's own list of commands that ran longer than its
`slowlog-log-slower-than` setting. It's the server's record, which `redis-pane` reads and, if you
ask, clears.

The list shows, for each entry, its age (`12s ago`), duration, command and client. A duration over
100 ms is coloured as a warning. A detail strip below shows the full command, the client and the
exact time in UTC.

An empty log can mean three different things, and the view says which: still fetching, the fetch
failed, or nothing has crossed the threshold.

## Keys

| Key | Does |
| --- | --- |
| `s` | Switch between most recent first and slowest first. |
| `r` | Fetch the whole log again. |
| `c` | Copy the selected command. |
| `d` | Reset the Slowlog (`SLOWLOG RESET`). |
| `↑` `↓` `PgUp` `PgDn` `Home` `End` | Move. |

`d` is a change like any other: you see the command, and `y` confirms. It's refused under every
reason for [Read-only Mode](../safety/read-only.md), including `replica`. There is no exception for a
write that touches no key.

On narrow terminals the client column goes first, then the age.

## On a Cluster

`SLOWLOG` is per node, so the view asks **every** node, replicas included, and merges the entries
into one list with a NODE column.

- The summary line counts the nodes, for example `SLOWLOG · 12 entries · 6 nodes · sort: recent`.
- A node that fails is named under the summary with the command and the reason. The other nodes'
  entries still show.
- `d` resets every node. The dialog reads `SLOWLOG RESET on 6 nodes`. If some fail, the report
  names them.
