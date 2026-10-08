# TLS

Managed Redis services (Upstash, Redis Cloud, Azure, ElastiCache with in-transit encryption)
usually refuse plaintext connections. There are three ways to ask for TLS:

- Use a `rediss://` URL (two `s`): `redis-pane --url rediss://default@your-host:6380`.
- Set `"tls": true` on a [Profile](profiles.md).
- Pass `--tls`.

```bash
redis-pane --host your-host --port 6380 --user default --tls
```

`--tls` only ever turns TLS on. There is no `--no-tls` to turn it off for a Profile or a
`rediss://` URL that asks for it.

For Sentinel and Cluster, use the `rediss-sentinel://` and `rediss-cluster://` schemes. See
[Sentinel](sentinel.md) and [Cluster](cluster.md).

If the connection fails with a TLS error, `redis-pane` exits and names the target and the
reason. Check that you used the TLS port, which is often not 6379.
