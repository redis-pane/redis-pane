# After the move to the `redis-pane` org: settings, fix-ups, community

Status: **planned** 2026-10-09. **Settings (S) done** the same day. The repo moved to `redis-pane/redis-pane`, and the docs are live at
https://redis-pane.github.io/redis-pane/ (#97, #98). This is the follow-up work.

## Decisions (user, 2026-10-09)

- **`--db` with a URL target is postponed.** It is recorded as an open Severity-1 item in
  `docs/UI_TASKS.md`. The README and the book's Limits page keep documenting it as a known bug.
  No code change.
- **The docs stay at `/redis-pane/`.** No `redis-pane.github.io` org site repo.
- **`main` is protected:**
  - a PR is required;
  - these checks are required:
    - `fmt · clippy · test`
    - `integration · real Redis (Docker)`
    - `perf · 1M-key budgets (release)`
    - `binary size (dist profile)`
    - `core has no I/O dependencies`

    `mdbook build · link check` is not required. It runs only when `book/**` changes, so
    requiring it would block every other PR.
  - no force-push and no deletion;
  - **admins can bypass**, for emergencies.
- **Community:** issue templates, Discussions, and an org profile README.

## Work

| # | What | Who | Delivers |
|---|---|---|---|
| S | Settings | Main session, via `gh api`, because it needs admin rights. No subagent | Repo description, homepage and topics. Discussions on. Branch protection as above. Org name, description and blog URL. The **org avatar** is uploaded by the user in the browser |
| 1 | Fix-ups ([`org-task1-fixups.md`](org-task1-fixups.md)) | Sonnet subagent | A misspelled Profile is an error. Help and `--help` gaps closed. The keybindings page stops promising overrides. `--db` recorded in UI_TASKS. Docs updated to match |
| 2 | Community files ([`org-task2-community.md`](org-task2-community.md)) | Sonnet subagent | Issue forms plus `config.yml`. The `redis-pane/.github` repo with `profile/README.md`. RELEASING gets a "first release from the org" check |

Tasks 1 and 2 touch different files, so they can run one after the other without waiting on each
other's review. Each is merged once every check passes.

## Shared rules

The same rules as [`docs-planning.md`](docs-planning.md):
- glossary words, from `CONTEXT.md`;
- accurate before complete;
- no tags;
- never remove Docker containers you didn't create;
- commits end with the `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` line, and PR
  bodies with the Claude Code line.

Generated references are regenerated with `UPDATE_DOCS=1` and never hand-edited.
