# The title bar tells the truth: `--db` over a URL (#106), target truncation (#107)

Status: **done** 2026-10-20. One task, one Sonnet subagent, one PR. It merges once every
check has finished and passed.

## Context

The title bar's target-and-Source readout is the whole mitigation for resolving a Connection
silently (ADR-0001, DESIGN §2: the title bar "carries all four things it must always carry:
Environment dot, target, database, and Source"). Two open issues break that promise:

- **#106 (Severity 1 in `docs/UI_TASKS.md`).** With a URL target (`--url`, `REDIS_URL`, or a
  Profile's `url`), `--db N` changes only the displayed target. `format_url`
  (`crates/core/src/resolve.rs`) appends `/N` to the redacted URL. `dial_from` dials the raw URL
  untouched, so the connection uses the URL's own database, or 0. A URL that already names a
  database shows both: `redis://host/2` under `--db 3` reads `…/2/3`. `from_profile` always
  passes `Some(db)`, so a Profile `url` shows a `/0` the URL may not have, even with no `--db`.
- **#107.** While Read-only Mode is on, the title bar cuts the target to `…90/0` even at about 100
  columns. `title_bar` (`crates/core/src/render/mod.rs`) trims the status readout only until
  Environment + Source fit. The target gets whatever is left, so a long readout
  (`READ-ONLY environment ^R  manual · never read`) eats the target first. DESIGN §2 allows the
  target to be truncated only at 70–89 columns.

## Decisions (user, 2026-10-20)

- **#106: the flag wins.** `--db` replaces the URL's database in what is dialled, the same way
  `--user` and `--password` already override. The title bar, `--print-target` and `--probe`
  all show the database actually used.
- **On a Cluster, `--db` other than 0 is refused.**
- **#106 and #107 ship in one PR.**

## Deliverables

### 1. `--db` over a URL (#106), in `crates/core/src/resolve.rs`

- **One effective database per Resolution**, by precedence:
  1. `--db`;
  2. a Profile's `db`, which already outranks its URL in `from_profile`'s
     `db_override.or(profile.db)`;
  3. the URL's own database (its path);
  4. `0`.

  If the config parser already refuses a Profile with both `url` and `db`, keep that refusal and
  skip rule 2 for URLs. Check `crates/core/src/config/mod.rs` first.
- **The dialled URL carries it.** Rewrite the URL's path to `/<db>`, replacing any existing
  database. Keep the scheme, userinfo (password included, since the dial URL is never shown),
  host list and query string (`redis-sentinel://h1:26379,h2:26379/0?sentinelServiceName=x`)
  intact. The path sits between the authority and `?`. Write a small pure helper with
  table-driven tests. Do not pull in a URL crate for this: the core's dependency set is
  deliberately small (ADR-0011's boundary only forbids I/O crates, but keep it lean).
- **The displayed target is the same URL, redacted** (`redact` already does that), with the same
  `/<db>`, never appended twice. A URL with no database and no `--db` displays `/0`, because the
  title bar always shows the database (DESIGN §2).
- **A Cluster refuses `--db` other than 0.**
  - **At resolve time, for a `redis-cluster://` or `rediss-cluster://` URL:** add a
    `ResolveError` variant with a message that says a Cluster has only database 0. Use the same
    exit path as `UnknownProfile` (exit code 3; check `main.rs`).
  - **For a plain URL or host that turns out to be a Cluster at connect time (ADR-0022):** first
    find out what happens today when a discrete `--host`/`--db 3` target is a Cluster. The URL
    case must behave identically. If today's behaviour silently uses db 0 while showing `/3`,
    that is the same bug: refuse it at the connection gate, with the same message. Say in the PR
    body which way it went.
- **`--print-target` and `--probe`** must show the same target and database as the title bar,
  and `--probe` must dial the same URL. They go through `Resolution`, so this should follow.
  Verify it.
- **Tests in `resolve.rs`**, displayed target *and* `dial_url` for each:
  - a URL with no database, with and without `--db`;
  - a URL with a database, with and without `--db`;
  - a URL with userinfo and a password: the display is redacted, the dial URL keeps the password;
  - a URL with a query string;
  - a Profile `url` with no `--db`: no phantom change to what is dialled;
  - `REDIS_URL` with `--db`;
  - a `redis-cluster://` URL with `--db 0` (fine) and `--db 3` (refused).
- **Integration test** (`crates/app/tests/integration.rs`, `#[ignore]`d like the rest): put a key
  in database 3 only, connect with a URL plus `--db 3`, and assert that the key is visible. If
  the suite has no reasonable entry point for a resolved target, test `connect_with` with the
  rewritten `dial_url` instead, and say so in the PR body.

### 2. Target truncation (#107), in `title_bar`

- **New yield order when the bar is too narrow:**
  1. the Cluster segment (as today);
  2. the **liveness** readout (`live` / `manual · never read` / …);
  3. the target, truncated from the left;
  4. the Read-only badge.

  Environment and Source never yield. Liveness goes first because the Viewer header carries it
  too (DESIGN §6.4). The READ-ONLY badge appears nowhere else, so it outranks the target, as it
  already outranks liveness.
- **In practice:** trim readout entries (from the end, as today) until the *whole* target also
  fits. Truncate the target only when the readout has nothing left to give that is below it.
  Keep the condition readout (`-LOADING`, `-OOM`, …) and READ-ONLY as they rank today.
- **Golden frames**, using the existing snapshot style:
  - `prod`, Read-only Mode on, a typical `127.0.0.1:6390/0` target at 120, 100, 90 and 80
    columns: the whole target at 100 and up, liveness dropped first;
  - one long target at 80 columns, where truncation still happens and Environment and Source
    survive;
  - update any existing snapshot that changes, and check each change is the intended one.

### 3. Docs, in the same PR

- **DESIGN §2:** state the yield order in the paragraph after "carries all four things", and keep
  the 70–89 row consistent with it.
- **The user guide:** `site/content/docs/connecting/databases.md` drops its known-bug note and
  states the rule (`--db` overrides a URL's database; a Cluster accepts only 0). Also fix
  anything else in `site/content/docs` that describes the old behaviour, and `limits.md` if it
  mentions it.
- **README:** remove the `--db` known-bug line.
- **`--help` text for `--db`:** if it changes, regenerate the CLI reference with `UPDATE_DOCS=1`
  (CLAUDE.md).
- **`docs/UI_TASKS.md`:** tick the `--db` item, record the decision and link #106.
- **`CHANGELOG.md` `Unreleased`:** a "Fixed" entry for each issue.
- **This plan:** mark the Status done.

## Shared rules

- Use the glossary's words, from `CONTEXT.md`. Profile, Connection, Source, Environment and
  Read-only Mode are distinct terms.
- `resolve` stays pure. Nothing in the core does I/O (`cargo tree` boundary check, CLAUDE.md).
- Never log or display a password. The dial URL is the only place one may survive.
- Generated references are never hand-edited.
- Never remove Docker containers you didn't create.
- No tags, and no repo-settings changes.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The PR
  body says `Fixes #106` and `Fixes #107`, and ends with
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Pass the body with
  `--body-file`.

## Checks

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all -- --check`
- `cargo tree -p redis-pane-core -e normal | grep -iE 'crossterm|tokio|fred'` must print nothing
- `pnpm --dir site build`
- The integration suite, because this touches the connection path: `cargo test -p redis-pane --
  --ignored --test-threads=1` (Docker)
