# Contributing to redis-pane

Thanks for wanting to help. This page covers how to build, test and send a change. Bug reports and
ideas are just as welcome: use the [bug report form](https://github.com/redis-pane/redis-pane/issues/new?template=bug.yml)
or [Discussions](https://github.com/redis-pane/redis-pane/discussions). If you are looking for a
first change, see the issues labelled
[`good first issue`](https://github.com/redis-pane/redis-pane/labels/good%20first%20issue).

## Prerequisites

- **Rust.** `rust-toolchain.toml` pins the stable channel with `rustfmt` and `clippy`, so
  [rustup](https://rustup.rs) installs the right thing the first time you build.
- **Node and pnpm**, only to work on the user guide in `site/`. The Node version is in
  `site/.nvmrc` and the pnpm version is in `site/package.json`.
- **Docker**, only for the integration suite.

## Build, run and test

From the repository root:

```bash
cargo build --workspace
cargo run -p redis-pane                      # the TUI; q or Ctrl-C quits
cargo run -p redis-pane -- --print-target    # resolve a target and exit, without connecting
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

There are two test suites, and they answer different questions:

- **The default suite** is `cargo test --workspace`. It runs the functional core and the
  golden-frame snapshots, and needs no Docker. Run it on every change.
- **The integration suite** runs against a real Redis in a container, so it needs Docker. Every
  test is `#[ignore]`d, so the default run skips it:

  ```bash
  cargo test -p redis-pane -- --ignored --test-threads=1
  ```

  Run it before trusting a change to the connection, the read path or live updates. Only these
  tests prove the app really arms live updates against a server.

To watch the app against live data, `scripts/` has a disposable Redis and a keyspace to put in it.
See [`scripts/README.md`](scripts/README.md).

## The generated docs

Two pages of the user guide are generated, never hand-edited: the keybindings reference (from the
keymap) and the command-line reference (from the `clap` definition). After changing a key or a
flag, regenerate them. CI fails if they are stale:

```bash
UPDATE_DOCS=1 cargo test -p redis-pane-core --test docs_reference
UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference
```

Every `json` block in the guide is parsed by the real config parser, so an example that would not
work cannot be published. To preview the guide, run `pnpm --dir site install` once, then
`pnpm --dir site dev`.

## The core and shell boundary

The code is a functional core with imperative shells around it. The core takes a message and
returns new state plus commands. It never touches the terminal, the network, the clock or
randomness, which are all injected, so a frame is a function of state alone. Terminal and Redis
I/O live in the shell (`crates/app`), and the render loop never does I/O. CI enforces the boundary:
this must print nothing.

```bash
cargo tree -p redis-pane-core -e normal | grep -iE 'crossterm|tokio|fred'
```

## Sending a change

- **One change per PR.** Small and focused is easier to review and to revert.
- **Update the docs in the same PR.** When a change alters what a user sees, types or configures,
  update its chapter in `site/content/docs`. When it diverges from the requirements or the design,
  update those docs too. They are the spec.
- **CI must be green** before a merge: formatting, clippy, the default tests, the docs build and
  the integration suite.
- Add a line to `CHANGELOG.md` under `Unreleased` for anything a user would notice.
- Use the words in the glossary. A Profile is not a Connection, and the distinctions are
  deliberate.

## Where decisions live

- [`CONTEXT.md`](CONTEXT.md) is the glossary. Read it before naming anything.
- [`docs/adr/`](docs/adr/) records decisions with the alternatives that were rejected. Check for
  one before changing connection, config or safety behaviour.
- [`docs/PRD.md`](docs/PRD.md) has the requirements, numbered R1.x to R7.x, so a commit can say
  "implements R2.1".
- [`docs/DESIGN.md`](docs/DESIGN.md) has the layout, the keymap and its growth rule, and the visual
  language.
- [`docs/PLAN.md`](docs/PLAN.md) has the milestones and the workspace layout.

[`CLAUDE.md`](CLAUDE.md) is the same material arranged as a guide for coding agents. You don't
need to read it.
