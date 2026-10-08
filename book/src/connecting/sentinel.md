# Sentinel

With Redis Sentinel, you ask a Sentinel node where the current master is, and it can move.
`redis-pane` does the asking for you: give it a Sentinel URL and it connects to whichever node is
the master.

## The URL

```text
redis-sentinel://[user:password@]sentinel-host:26379[/db]?sentinelServiceName=mymaster
```

- The host and port are a **Sentinel** node, not the master.
- `sentinelServiceName` is required. It is the name your Sentinel configuration gives the master
  (`mymaster` in the examples). Without it `redis-pane` exits with
  `Invalid or missing sentinel service name query parameter`.
- To list more Sentinel nodes, repeat `node=host:port`:
  `?sentinelServiceName=mymaster&node=sentinel-2:26379&node=sentinel-3:26379`.
- Use `rediss-sentinel://` for [TLS](tls.md).

```bash
redis-pane --url 'redis-sentinel://sentinel-1.example.com:26379?sentinelServiceName=mymaster'
```

Quote the URL: the `?` and `&` mean something to your shell.

As a Profile:

```json
{
  "profiles": {
    "prod-sentinel": {
      "url": "redis-sentinel://sentinel-1.example.com:26379?sentinelServiceName=mymaster&node=sentinel-2.example.com:26379",
      "passwordEnv": "PROD_REDIS_PASSWORD",
      "env": "prod"
    }
  }
}
```

## What to expect

- The title bar shows the Sentinel URL you gave, with any password masked. Check it with
  `--print-target` before you connect.
- The Environment is inferred the same way as for any URL: a loopback Sentinel host is `local`,
  anything else is `unknown`. Set `env` on a Profile to say what it really is.
- The password and username you give are used for the master. Separate credentials for the
  Sentinel nodes themselves (`sentinelUsername`, `sentinelPassword`) are not supported.
- Browsing, live updates, editing and the server views run against the master. Monitor and
  Pub/Sub open their own connections to it.

If the master moves, the connection drops and `redis-pane` reconnects through Sentinel. As with
any reconnect, the open key is re-armed for live updates before the header says `● live` again.
See [Live updates](../viewing/live-updates.md). This release has been checked against a Sentinel
resolving to a master, but not against a failover in progress.
