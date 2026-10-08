# Docs task 2: the book

Part of [`docs-planning.md`](docs-planning.md). Read its shared rules first. Builds on task 1's
generated pages in `book/src/reference/`.

## Context

The user guide today is `ALPHA.md` plus scattered README paragraphs. Many behaviours a user
meets are explained only in DESIGN or the ADRs:
- the four Environments and Read-only Mode's reasons;
- what the live/degraded header states mean;
- the clipboard over SSH;
- the scan cap;
- Sentinel.

This task builds the user documentation as an mdBook site.

## Deliverables

1. **`book/book.toml`**:
   - title "redis-pane";
   - `src = "src"`;
   - the HTML output's `git-repository-url`, an edit-URL template, and search on.

   Use mdBook's default theme. Don't add CSS beyond what's needed.
2. **`book/src/SUMMARY.md`** and the chapters below. Keep one topic per page.
   - **Introduction** (`README.md` of the book): what it is, who it's for, and the comparison
     table. Task 3 finalises the table, so keep the two identical.
   - **Getting started:**
     - Installation: release binaries, the installer scripts, unsigned-binary first run, from
       source.
     - Quick start.
   - **Connecting:**
     - How the target is chosen: precedence, `REDIS_URL` used wholesale, the title-bar
       target-and-Source readout, `--print-target`, `--probe`.
     - Profiles and the config location.
     - Passwords and secrets: `passwordEnv`, `passwordCommand`, a literal password and the file
       permission check, the `--password` warning.
     - TLS.
     - Sentinel: confirm the URL form in `resolve.rs` and with a run before you document it.
     - Cluster.
     - Databases (`--db`, fixed at launch).
   - **Environments and safety:**
     - `local` / `staging` / `prod` / `unknown`, and how each is inferred;
     - Read-only Mode and its reasons (`environment`, `replica`, `user`) and `⌃R`;
     - command previews;
     - confirmation that scales (the typed count on `prod` / `unknown` bulk delete).
   - **Browsing:**
     - SCAN streaming, tree and flat, filter (substring, glob, fuzzy where it exists), sort
       orders;
     - large keyspaces: the cap banner and `rebuilding N%`;
     - session restore.
   - **Viewing values:**
     - one section per type, including Stream, JSON and Binary;
     - live updates: the header's states and what "degraded to manual" means, with Refetch `r`;
     - the Viewer holding a deleted key;
     - copying: `c`, `C`, and pbcopy vs. OSC 52.
   - **Editing:**
     - inline editing per type;
     - TTL;
     - rename and duplicate (`R`, `D`, Cluster cross-slot rules);
     - field and member rename;
     - bulk delete (marks, the typed count, cancelling, "loaded keys only").
   - **Server views:** Dashboard, Monitor (its cost, and the ask on `prod` / `unknown`),
     Pub/Sub (sharded), Slowlog. Each page has a "On a Cluster" section.
   - **Appearance:**
     - built-in themes;
     - custom themes, with the token list from `crates/core/src/theme`;
     - ASCII fallback (`--ascii` / `--unicode` / the locale rule);
     - `NO_COLOR`;
     - terminal sizes: the breakpoints in DESIGN §2 and the sub-70 stack.
   - **Reference:**
     - `keybindings.md` and `cli.md` (from task 1, untouched).
     - `configuration.md`: every field of the config file, with type and default. Use real
       examples (task 1's test parses every ```` ```json ```` block), plus one refused example.
       Unknown fields are an error, and say why.
     - `environment.md`: `REDIS_URL`, `REDIS_HOST` / `REDIS_PORT` / `REDIS_USER` /
       `REDIS_PASSWORD`, `NO_COLOR`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, the SSH variables.
   - **Troubleshooting and FAQ:**
     - unsigned binaries;
     - an empty paste over SSH (terminal list, tmux setting);
     - boxes instead of glyphs;
     - "not live";
     - config refused (permissions, an unknown field);
     - Redis older than 6.0;
     - a Cluster node gone.
   - **Limits and non-goals:**
     - move-across-db;
     - `$EDITOR`;
     - the console (R5.2);
     - the Streams timeline;
     - bulk marking only of loaded keys;
     - `Esc` doesn't cancel a rebuild;
     - plus the README's "not planning to build" list.
   - **Contributing:** a short page pointing to `CLAUDE.md`, `CONTEXT.md`, `docs/` and
     `scripts/README.md`.
3. **`TESTING.md`** at the root (≤ 80 lines):
   - the current beta;
   - a short "what to try in this beta" list linking into the book;
   - "something to point it at" (local Redis, `scripts/` fixtures);
   - how to report (what to include).
4. **Delete `ALPHA.md`.** Update every link to it: README, `scripts/README.md`, `docs/`,
   `CLAUDE.md`, and the release and close-out procedures.
   (`grep -rn ALPHA.md --exclude-dir=target`).
5. **`.github/workflows/docs.yml`**:
   - **On PRs and pushes touching `book/**`:** install mdBook (pinned version, with
     `taiki-e/install-action` or `peaceiris/actions-mdbook`), run `mdbook build book`, and run a
     link check over the built HTML (`lycheeverse/lychee-action`, offline or internal links only,
     so CI doesn't flake on external sites).
   - **On push to `main`:** deploy to Pages with `actions/upload-pages-artifact` and
     `actions/deploy-pages`, with `permissions: pages: write, id-token: write`.
   - Pages itself is enabled by the user, not by this task. The deploy job must not fail the
     workflow on `main` while Pages is off, so make it a separate job.
6. **Images:** reuse the root GIFs (`demo.gif`, `navigation-demo.gif`, `monitor-demo.gif`) by
   copying them into `book/src/images/`, where mdBook serves them. Keep the root copies, because
   the README uses them.
7. **CLAUDE.md:** in the Commands section, add `mdbook serve book` and the `UPDATE_DOCS=1`
   regeneration command. Extend "update the doc in the same change" to include the book.

## Rules this binds

- Write in the second person, plainly, with short paragraphs and a key in backticks wherever an
  action is named. Every key and flag must exist in the generated references.
- Migrate, don't duplicate. When a topic moves to the book, the README links to it rather than
  repeating it.

## Testing

- `cargo install mdbook --locked` locally if it's missing (it lives in `~/.cargo`). Then:
  - `mdbook build book` has no warnings;
  - a local link check is clean;
  - `cargo test --workspace` passes, including task 1's config-example test against the expanded
    `configuration.md`.
- Spot-check by hand that a Sentinel URL resolves, using `--print-target`.

## Out of scope

The README rewrite and the CHANGELOG (task 3), and any product change.
