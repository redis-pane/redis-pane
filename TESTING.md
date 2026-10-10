# Testing the beta

Thanks for trying `redis-pane`. This is `0.1.0-beta.4`. It's early, so expect rough edges, and
please say so when you hit one.

The full guide is the [user guide](https://redis-pane.github.io/redis-pane/docs). Its source is in
[`site/content/docs`](site/content/docs/index.md).

## Install

Follow [Installation](site/content/docs/getting-started/installation.md). The builds are unsigned, so
your OS may ask you to confirm the first run.

## Something to point it at

Any Redis 6.0 or newer works, and so does Valkey. If you don't have one:

```bash
brew install redis && brew services start redis
```

Or use a free instance on [Upstash](https://upstash.com) or [Redis Cloud](https://redis.io). Both
work, TLS included.

From a checkout, `scripts/` gives you a disposable Redis and a keyspace to put in it. See
[`scripts/README.md`](scripts/README.md). `python3 scripts/fixtures.py --flush -n 1000000` loads
a million keys, and `python3 scripts/churn.py` keeps them changing.

## What to try

- **Live updates.** Open a key, then change it from another terminal with
  `redis-cli HSET the-key field value`. The screen should update with no keypress. If the
  header never says `● live`, report it. [Live updates](site/content/docs/viewing/live-updates.md)
- **Editing.** Edit a value, change a TTL, rename and duplicate a key, and bulk-delete a few.
  Check that each preview shows the command you expected.
  [Editing values](site/content/docs/editing/values.md)
- **Safety.** Connect to something that isn't loopback. It should start in Read-only Mode and say
  why. [Environments](site/content/docs/safety/environments.md)
- **The server views.** `g d`, `g m`, `g p` and `g s`.
  [Dashboard](site/content/docs/server/dashboard.md)
- **A very large keyspace.** Nothing should freeze while it scans, toggles the tree, changes the
  sort or filters. A visible stall is worth reporting.
  [Large keyspaces](site/content/docs/browsing/large-keyspaces.md)
- **A Cluster or Sentinel target.** Kill a node and watch what the title bar and the server views
  say. [Cluster](site/content/docs/connecting/cluster.md), [Sentinel](site/content/docs/connecting/sentinel.md)
- **Looks.** Try `--theme light`, `--ascii`, and resize the terminal.
  [Terminal sizes](site/content/docs/appearance/terminal-sizes.md)
- **Copying over SSH.** Press `c` and paste. [Copying](site/content/docs/viewing/copying.md)

What isn't built yet is on the [limits page](site/content/docs/limits.md). Check it before reporting
something as new.

## Reporting something

Use the [bug report form](https://github.com/redis-pane/redis-pane/issues/new?template=bug.yml).
It asks for the details below. For questions and ideas, use
[Discussions](https://github.com/redis-pane/redis-pane/discussions). You can also message me
directly. Please include:

- what you were doing, what you expected, and what happened instead
- the version (`redis-pane --version`)
- what you were connected to: Redis version, and whether it was local, Upstash, Redis Cloud or
  something else
- your terminal and whether you were over SSH
- the output of `redis-pane --print-target` and `redis-pane --probe` for that target, with any
  password removed

Genuinely appreciate you trying this out.
