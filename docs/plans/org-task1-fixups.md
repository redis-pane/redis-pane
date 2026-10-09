# Org task 1: fix-ups found while writing the docs

Part of [`org-followups-planning.md`](org-followups-planning.md).

## Context

Docs tasks 1 and 2 (#94, #95) found small gaps between the code and what a user is told. This task
closes all of them except `--db`, which the user postponed.

## Deliverables

1. **A misspelled Profile is an error, not a silent fallback.**
   - **Today:** `redis-pane --profile typo` and `redis-pane typo` fall through to the next rule in
     `resolve_target` (`crates/core/src/resolve.rs`, rule 1: `if let Some(name) = &flags.profile
     && let Some(profile) = …`), and connect to localhost with Source `from default`.
   - **The rule:** a Profile named on the command line that doesn't exist is an error that names
     it and lists the available Profiles. When no config file exists, it says that instead.
   - **Exit code 3**, matching `--theme` (changed in review; the plan first said 2). Originally: the same as other startup refusals; check `main.rs`/`connect_or_exit` for the
     existing convention.
   - **Matches the spirit of ADR-0001 and ADR-0002:** a typo in a hand-authored config is never
     silently dropped, and the same goes for a typo on the command line.
   - **A missing `defaultProfile`** is already a parse error. Leave it as is.
   - **Where it lives:** keep `resolve` pure. Return the error through the existing Resolution
     path, or change `resolve` to return a `Result`, whichever is smaller. Update every caller,
     including `--print-target`.
   - **Tests:** unit tests in `resolve.rs` for the flag form, the positional form, no config file,
     and a correct name still resolving. Also an app-level test, if one exists for startup exit
     codes.
2. **Help gaps (`crates/core/src/help.rs`):**
   - The Key list context shows `⏎` (open into the Viewer) and `←`/`h` (collapse / parent).
   - `⌃←`/`⌃→` (narrow / widen the keys pane) and `⌃Y` (redo, where it applies) appear in the
     context where they work.
   - `⌃C` quit shows next to `q`, for example by merging the keys as `q ⌃C` the way `? F1` is
     merged.
   - Enter is spelled one way everywhere, using `key_label`'s `⏎`. Make `Tab` and `↑` in
     `help.rs` consistent with `key_label` too.
   - The hint bar is the first N help rows, so check the golden frames. Update intended changes
     with the existing golden workflow, and review each diff: no unrelated frame may change.
3. **`--help` (`crates/app/src/cli.rs`):**
   - `--host` and `--port` get descriptions.
   - Enable the doc comment on resolution order as `long_about`, or move it into
     `after_long_help`.
   - `--db`'s help drops "ADR-0005" and says plainly that the database is fixed at launch.
   - Don't change `--db`'s behaviour.
4. **The keybindings page** (the generator in `crates/core/tests/docs_reference.rs`): remove the
   claim about bindings "you have overridden". Keys can't be rebound from config (DESIGN §4
   reserves it, and nothing reads it). Regenerate both reference pages with `UPDATE_DOCS=1`.
5. **Record the `--db` bug** as an open `- [ ]` item under Severity 1 in `docs/UI_TASKS.md`:
   - **The symptom:** `--db` with a URL target (`--url`, `REDIS_URL`, a Profile's `url`). The
     title bar shows `/N`, but the connection uses the URL's database or 0. `format_url` appends
     `/N` to the displayed target, while `dial_from` dials the raw URL. A URL that already carries
     `/2` displays `/2/3`.
   - **Decision pending,** with the two candidates: the flag overrides the URL's database (on a
     Cluster, only 0 is allowed), or the combination is refused.
   - **Postponed by the user** on 2026-10-09.
6. **Docs:**
   - Remove "a misspelled `--profile` falls back to localhost" from the README's "What it doesn't
     do" and from the book (Limits, Troubleshooting, Connecting → How the target is chosen, and
     anywhere `grep -rn "profile" book/src README.md` shows it). Document the new error instead.
   - Keep the `--db` caveat, and point it at the UI_TASKS item.
   - Add a CHANGELOG `Unreleased` entry for each user-visible change.

## Testing

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`, including the docs-reference tests after regeneration.
- `mdbook build book` with no warnings.
- **By hand:**
  - `cargo run -p redis-pane -- --profile nope --print-target` exits 3 with the message;
  - with a valid Profile, it still resolves;
  - `--help` shows host and port descriptions.
- The Docker integration suite is required if the startup path in `main.rs` changes:
  `cargo test -p redis-pane -- --ignored --test-threads=1`.

## Out of scope

`--db` behaviour, keybinding overrides, and any new feature.
