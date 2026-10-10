---
title: "Contributing"
---

Working on the code? These are where to start, all in the
[repository](https://github.com/redis-pane/redis-pane):

- **`CLAUDE.md`** has the architecture notes, the commands to build and test, and the
  conventions. Despite the name, it's the guide for any contributor.
- **`CONTEXT.md`** is the glossary. Several terms are deliberately distinct (a Profile isn't a
  Connection), and the distinctions matter.
- **`docs/`** holds the design record: the product requirements (`PRD.md`), the plan (`PLAN.md`),
  the screens and keymap (`DESIGN.md`), and `adr/`, the decisions with the alternatives that were
  rejected. Check for an ADR before changing connection, config or safety behaviour.
- **`scripts/README.md`** describes a disposable Redis and a keyspace to put in it, with a
  churn script that keeps the data moving, so you can watch live updates and `SCAN` instead of
  reasoning about them.

## Working on this book

The source is in `book/src`. To preview it:

```bash
cargo install mdbook --locked
mdbook serve book
```

Two pages are generated from the code: the [keybindings](./reference/keybindings.md) and the
[command-line options](./reference/cli.md). Don't edit them by hand. After changing a key or a
flag, regenerate them:

```bash
UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference
UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference
```

CI fails if either is out of date. The JSON examples in
[Configuration](./reference/configuration.md) are parsed by the real config parser, so an example
that wouldn't work can't be published.
