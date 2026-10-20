---
title: "Databases"
---

A session has **one Connection to one database, fixed when you launch**. There is no database
switcher, no `SELECT` and no tabs. To look at another database or another server, start another
`redis-pane` in another terminal.

This is on purpose. It keeps the title bar's answer to "what am I connected to?" true for the
whole session.

## Choosing a database

One database applies, whichever way you name the target. The first of these that is set wins:

1. `--db N`
2. The Profile's `"db": N`
3. The database in the URL's path: `redis://host:6379/3`
4. `0`

```bash
redis-pane --host localhost --db 3
redis-pane --url redis://localhost:6379/3
redis-pane --url redis://localhost:6379/2 --db 3   # database 3
```

`--db` replaces a URL's database, the way `--user` and `--password` replace the ones a URL
carries. The scheme, the credentials, the hosts and the query string of the URL are kept. The title
bar, `--print-target` and `--probe` all show the database the connection uses, so
`redis://localhost:6379/3` is what you read for the last line above.

On a [Cluster](./cluster.md) there is only database 0. `--db 0` is accepted. Any other database,
whether from `--db`, a Profile's `db` or the URL, is refused, with exit code `3` for a
`redis-cluster://` URL. A plain `redis://` URL that turns out to name a Cluster node is refused when
it connects, with the server's own `SELECT is not allowed in cluster mode`.
