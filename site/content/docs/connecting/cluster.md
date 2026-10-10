---
title: "Cluster"
---

`redis-pane` browses a Redis Cluster. Point it at the whole Cluster, or at any one node.

```bash
redis-pane --url redis-cluster://cluster-node-1.example.com:6379
redis-pane --url redis://cluster-node-1.example.com:6379
```

The first form says "this is a Cluster". The second is a plain URL, and `redis-pane` notices the
node is cluster-enabled and reconnects as a Cluster. Use `rediss-cluster://` for [TLS](./tls.md).

The title bar adds the shape of the Cluster after the target, for example
`cluster · 3 primaries · 6 nodes`. When the bar is tight, the shape is the first thing to go.

## What changes on a Cluster

- **The key list covers every primary.** `SCAN` runs against each primary and the results are
  merged. A topology change in the middle of a scan (a failover, a slot move) stops the scan and
  says so. The keys you have stay, and `r` rescans.
- **The open key is live** on whichever node owns its slot, through failovers and slot moves.
- **Edits go to the owner of the key.**
- **There is only database 0.** The title bar doesn't show a database, and
  `--db` has no use here.
- **Rename and duplicate** have a cross-slot rule. See
  [Rename and duplicate](../editing/rename-duplicate.md#on-a-cluster).
- **Bulk delete** sends one `DEL` per key, so keys in different slots delete without trouble.
- **`C` copies a Cluster-aware command**, `redis-cli -c -h <host> -p <port> ...`. The `-c`
  makes `redis-cli` follow redirects.
- **The server views read every node.** See [Dashboard](../server/dashboard.md#on-a-cluster),
  [Monitor](../server/monitor.md#on-a-cluster), [Pub/Sub](../server/pubsub.md#on-a-cluster) and
  [Slowlog](../server/slowlog.md#on-a-cluster).

## If a node goes away

The rest of the Cluster keeps working. Views that read every node show the node that failed and
the command that failed, and carry on with the others. See
[Troubleshooting](../troubleshooting.md#a-cluster-node-is-gone).
