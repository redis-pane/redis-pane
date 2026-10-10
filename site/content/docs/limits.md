---
title: "Limits and non-goals"
---

## Not built yet

- **Moving a key to another database.** One Connection means one database. See
  [Databases](./connecting/databases.md).
- **Opening a value in your own `$EDITOR`.** Values are edited inline, and values over 200 KB can't
  be edited. See [Editing values](./editing/values.md).
- **A Redis console.** There is no place to type a raw command. `redis-cli` is one keystroke away
  in the terminal you're already in. A command palette shipped briefly and was withdrawn, because
  every action it listed already had a key.
- **A Streams timeline.** A Stream is shown as a list of its newest 500 entries, and is read-only.
- **Rebinding keys.** The keys are fixed in this release.
- **Marking a whole tree group, or everything a filter matches, for deletion.** You can mark
  only keys you've loaded, one at a time. A marked key that a filter hides is still deleted.
  See [Bulk delete](./editing/bulk-delete.md).

## Known rough edges

- **`Esc` doesn't cancel a rebuild** of a large list. The old list stays usable, and changing the
  filter or sort again replaces the rebuild.
- **Restoring a saved session in tree mode at a million keys** rebuilds once at startup, so
  it can pause briefly.
- **Filters with `*` or `?`** aren't on the fast path, so on a very large list they rebuild
  slower than plain-text filters. Case-insensitive matching handles ASCII letters only.
- **No fuzzy filter** you can switch on. A filter is a substring or a glob.
- **Keys beyond 2,000,000** aren't loaded. See [Large keyspaces](./browsing/large-keyspaces.md).
- **RedisJSON module keys** are listed with the type `json`, but haven't been checked here.
- **Sentinel** has been checked resolving to a master, but not through a failover.
- **Older than Redis 6.0 and RESP2** aren't supported. You get a clear message, not a crash.

## Not planning to build

`redis-pane` is not:

- a server manager,
- an alerting or paging tool, or
- a metrics store. The Dashboard reads `INFO` live and keeps nothing but an in-session
  sparkline. It doesn't retain history across a restart, and it notifies no one,
- a replacement for `redis-cli` in scripts.

It also has no multi-server workspace, no tabs or database switcher, no plugins, and no
export or import. It's one Connection at a time, kept simple.
