---
title: "What's next"
description: The open issues that shape where redis-pane goes next.
---

These are the open issues that matter most right now. The order may change, and nothing here has a
date or is a promise. The [issue tracker](https://github.com/redis-pane/redis-pane/issues) is the
source of truth.

## Highest priority

- **Windows.** [#138](https://github.com/redis-pane/redis-pane/issues/138) verifies the build and the
  installer on Windows, and fixes where the config file, the saved session and `passwordCommand`
  look. It comes first, because Windows stays experimental until it is done.

## Wanted

- **Copy a nested JSON value.**
  [#122](https://github.com/redis-pane/redis-pane/issues/122) is a key that copies the selected object
  or property of a JSON value, while `c` keeps copying the whole value.
- **Finer bulk-delete progress.**
  [#112](https://github.com/redis-pane/redis-pane/issues/112) makes the `deleting X of N` line move for
  deletes of fewer than 500 keys, instead of jumping from `0` to `N`. It is labelled `good first
  issue`.

## Decisions pending

- **Keybinding overrides.** [#110](https://github.com/redis-pane/redis-pane/issues/110) asks whether
  overriding keys from the config file is planned, or whether the design documents should say it
  isn't.
- **A fuzzy filter.** [#109](https://github.com/redis-pane/redis-pane/issues/109) asks whether to
  offer a fuzzy filter you can switch on (it exists in the core, but nothing reaches it), or to
  remove it.

## More

- [Limits and non-goals](./limits.md) lists what is deliberately not built.
- Have an idea? Start a thread in
  [Discussions](https://github.com/redis-pane/redis-pane/discussions).
- Want to help? Read
  [CONTRIBUTING.md](https://github.com/redis-pane/redis-pane/blob/main/CONTRIBUTING.md), then pick
  from the [open issues](https://github.com/redis-pane/redis-pane/issues).
