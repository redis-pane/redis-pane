# Databases

A session has **one Connection to one database, fixed when you launch**. There is no database
switcher, no `SELECT` and no tabs. To look at another database or another server, start another
`redis-pane` in another terminal.

This is on purpose. It keeps the title bar's answer to "what am I connected to?" true for the
whole session.

## Choosing a database

Which way depends on how you name the target:

| You use | Choose the database with |
| --- | --- |
| `--host` and `--port`, or a Profile with `host` and `port` | `--db N`, or `"db": N` in the Profile |
| `--url`, `REDIS_URL`, or a Profile with `url` | The URL path: `redis://host:6379/3` |

```bash
redis-pane --host localhost --db 3
redis-pane --url redis://localhost:6379/3
```

> **`--db` doesn't change a URL.** With `--url`, a Profile's `url`, or `REDIS_URL`, `--db` and a
> Profile's `db` are not applied to the connection, even though the title bar then shows the
> number. Put the database in the URL itself. This is a known bug, recorded in
> [`docs/UI_TASKS.md`](https://github.com/redis-pane/redis-pane/blob/main/docs/UI_TASKS.md).

On a [Cluster](cluster.md) there is only database 0.
