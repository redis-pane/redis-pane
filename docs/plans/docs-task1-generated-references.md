# Docs task 1: generated references

Part of [`docs-planning.md`](docs-planning.md). Read its shared rules first.

## Context

CLAUDE.md says "keybindings are data": the keymap, the help overlay and the hint bar all read
from one source. The docs don't. The README's and ALPHA's key lists are hand-written and have
already drifted. This task makes the user-facing references come from the same source, and fails
CI when they are stale. lazygit does the same for `docs/keybindings`.

## Deliverables

1. **`book/src/reference/keybindings.md`, generated.**
   - **Source:** `Keymap::default()` (`crates/core/src/keymap/mod.rs`: `bindings()`, `chords()`,
     `key_label`) and the contextual help model (`crates/core/src/help.rs`: `HelpContext`,
     `here`, `everywhere`, `HelpRow`).
   - **Content:** one section per context a user meets:
     - Everywhere (global)
     - Key list
     - Value pane: one section per type (String, Hash, List, Set, Sorted set, Stream, JSON,
       Binary), with the cursor on
     - Editor
     - Confirm
     - Chords (`g` …)
     - Dashboard (and its node view)
     - Monitor
     - Pub/Sub
     - Slowlog
   - **Each section** is a two-column table, `Key | Action`.
   - **How the rows are produced:** build a representative `State` for each context, the same
     way the help/golden tests build theirs. Reuse their fixtures; don't invent new
     constructors if test helpers exist. Then call `help::here` (plus `help::everywhere` once).
   - **Refusals are not printed.** The page describes bindings, not one state's refusals, so
     build each state with Read-only Mode lifted, or drop `refused`.
   - **Header:** "Generated from the keymap; do not edit. Regenerate with `UPDATE_DOCS=1 cargo
     test -p redis-pane-core --test docs_reference`." Add a one-line intro per section.
2. **`book/src/reference/cli.md`, generated** from clap. Expose the `Cli` command, for example
   with a `pub fn cli_command() -> clap::Command` in the app crate. Render `render_long_help()`
   inside a fenced block, and add a short intro on resolution order: flags → Profile →
   environment → `127.0.0.1:6379`. The header is the same "do not edit" note.
3. **The golden check:** a test per page.
   - It renders the Markdown and compares it to the checked-in file.
   - On a mismatch it fails with "docs reference out of date — run with `UPDATE_DOCS=1`".
   - With `UPDATE_DOCS=1` set, it writes the file instead.
   - It runs in the default Docker-free `cargo test --workspace`.
   - The pages must be byte-stable: no timestamps, no HashMap order.
4. **Config examples can't rot:** a test in the core crate.
   - It reads `book/src/reference/configuration.md` (task 2 writes the prose; this task creates
     the file with at least one complete example).
   - It extracts every ```` ```json ```` block and parses each one with the real config parser
     (`crates/core/src/config/mod.rs`), so an unknown or misspelled field fails CI.
   - A block fenced as ```` ```json,invalid ```` is expected to fail, which lets the page show a
     refused example.

Create `book/` only as far as these files need. Task 2 builds the rest of the book around them.

## Rules this binds

- The core stays I/O-free in its library code. Reading and writing files belongs only in tests,
  under `crates/core/tests/`, so the `boundary` job is unaffected.
- No product behaviour changes. If generating exposes a wrong label or a missing binding,
  **list it in the PR**; don't fix it here unless it's a one-line label typo.

## Testing

- fmt, clippy with `-D warnings`, and `cargo test --workspace` all pass.
- **Prove the check works:** temporarily change a binding's label or key, see the test fail with
  the message, then revert. Say in the PR that you did.
- `cargo run -p redis-pane -- --help` matches `cli.md`.

## Out of scope

The rest of the book, the README and mdBook tooling (task 2).
