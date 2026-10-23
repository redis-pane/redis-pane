---
title: "Contributing"
---

Working on the code? Start with
[`CONTRIBUTING.md`](https://github.com/redis-pane/redis-pane/blob/main/CONTRIBUTING.md) in the
[repository](https://github.com/redis-pane/redis-pane). It covers building, the two test suites,
the generated docs and how to send a change. It points to:

- **`CONTEXT.md`**, the glossary. Several terms are deliberately distinct (a Profile isn't a
  Connection), and the distinctions matter.
- **`docs/`**, the design record: the product requirements (`PRD.md`), the plan (`PLAN.md`), the
  screens and keymap (`DESIGN.md`), and `adr/`, the decisions with the alternatives that were
  rejected. Check for an ADR before changing connection, config or safety behaviour.
- **`scripts/README.md`**, which describes a disposable Redis and a keyspace to put in it, with a
  churn script that keeps the data moving, so you can watch live updates and `SCAN` instead of
  reasoning about them.

Issues labelled
[`good first issue`](https://github.com/redis-pane/redis-pane/labels/good%20first%20issue) are a
good place to begin.

## Working on this guide

Pages are plain `.md` files in `site/content/docs`, ordered by each folder's `meta.json`. To
preview the guide:

```bash
pnpm --dir site install
pnpm --dir site dev
```

Then open http://localhost:3000/redis-pane. Node is pinned in `site/.nvmrc`, and pnpm by
`site/package.json`.

Two pages are generated from the code: the [keybindings](./reference/keybindings.md) and the
[command-line options](./reference/cli.md). Don't edit them by hand. After changing a key or a
flag, regenerate them:

```bash
UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference
UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference
```

CI fails if either is out of date. Every `json` code block in the guide, such as the examples in
[Configuration](./reference/configuration.md), is parsed by the real config parser, so an example
that wouldn't work can't be published.
