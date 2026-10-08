# Documentation: reorganise around what a user can do

Status: **planned** 2026-10-08, after `0.1.0-beta.4`. Three tasks. Each task gets its own plan
doc and its own Sonnet subagent run, and is merged before the next starts. This is the same
process as M4–M6.

## Context

The README grew one paragraph per milestone. Its "Status" section is the only feature list, and
it reads like a changelog. A reader can't quickly tell what redis-pane does, what it can't do,
or how it compares to the tools they already use. Other problems:
- The real user guide is `ALPHA.md`, which is named for a phase that is over and repeats the
  install section.
- There is no keybinding reference. The two hand-written key lists (README, ALPHA) have already
  drifted from the keymap.
- There is no reference for the config file or the environment variables.
- Sentinel works but isn't mentioned anywhere a user would look.
- User-facing docs and internal docs (PRD, PLAN, ADRs, plans) share `docs/`.

**Compared:** lazygit, k9s, yazi, atuin, gitui.
- **The common pattern:** a short README (pitch, demo, grouped feature bullets, install, link to
  docs), with full docs elsewhere.
- **What we borrow:**
  - lazygit's keybindings page, generated from code.
  - gitui's comparison table against alternatives and its "known limitations" list.
  - yazi's grouped, scannable feature list.
  - k9s's config reference built from real examples.

## Decisions (user, 2026-10-08)

- **Full docs are an mdBook site on GitHub Pages**, built from `book/` at the repo root.
  `docs/` stays as it is, for contributors, so no internal link breaks.
- **The keybindings reference is generated from the keymap** and checked in CI, so it cannot
  drift. CLI options are generated from clap the same way.
- **ALPHA.md is split** into the book plus a short `TESTING.md`, and then deleted.
- **The README gets a capability comparison table** against `redis-cli` and RedisInsight. Every
  cell is a claim we have checked.
- **Process:** I merge each PR once it is green and start the next. I stop only on a failure or a
  question that needs the user. **Enabling GitHub Pages** is a repo setting, so I ask the user
  before changing it.

## Target structure

```
README.md      ~150 lines: the front door
TESTING.md     what to try in this beta, how to report it
CHANGELOG.md   per release, newest first
book/          mdBook source → https://vinodsantharam.github.io/redis-pane
docs/          unchanged: PRD, PLAN, DESIGN, ADRs, plans
```

## Tasks

| # | Task | Plan doc | Delivers |
|---|---|---|---|
| 1 | Generated references | [`docs-task1-generated-references.md`](docs-task1-generated-references.md) | Keybindings and CLI pages generated from code, behind a CI check; config examples parsed by the real parser |
| 2 | The book | [`docs-task2-book.md`](docs-task2-book.md) | `book/` with every chapter, `TESTING.md`, `ALPHA.md` removed, and a `docs.yml` workflow (build and link check on PRs, Pages deploy on `main`) |
| 3 | README + CHANGELOG | [`docs-task3-readme.md`](docs-task3-readme.md) | README rewritten to the outline, the comparison table verified, `CHANGELOG.md` backfilled, and the release procedure updated |

Order: 1 → 2 → 3. The book embeds task 1's generated pages, and the README links into the book.

## Shared rules, binding on every task

- **Accurate before complete.** Every key, flag, config field and behaviour named in prose is
  checked against the code: the generated references, `crates/core/src/config/mod.rs`,
  `resolve.rs` and `main.rs`. If the code and an old doc disagree, the code wins, and the
  disagreement is listed in the PR body.
- **Use the glossary's words** (`CONTEXT.md`): Profile, Connection, Environment, Read-only Mode,
  Viewer, Refetch, Loaded set and the rest. User text does not cite ADR numbers. Link to an ADR
  only where the "why" helps a user.
- **No claim we haven't measured.** Numbers come from the perf harness, the release assets or a
  run, and the PR says which.
- **Never remove a Docker container you didn't create.** `redis-pane-dev` and
  `redis-pane-cluster` belong to the user.
- **No release tag.** Nothing here changes the version.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. PR bodies end
  with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
