# Docs verification, then `0.1.0-beta.5`

Status: **planned** 2026-10-20. Two tasks. Each runs on one Sonnet subagent, and each PR merges
once every check has finished and passed. Task 2 starts after task 1 is merged.

## Context

All of M0–M6 has shipped, plus the M2 tail (`0.1.0-beta.4`). Since `beta.4`, #93–#114 reworked the
docs (the Fumadocs site, landing page, Tier 1 demos), moved the repo to the `redis-pane` org,
added community files and the brand, and fixed several things a user sees (#100, in `CHANGELOG.md`
under `Unreleased`). None of that is released. A documentation check on 2026-10-20 found drift,
listed in task 1.

## Decisions (user, 2026-10-20)

- Fix the doc drift first, in one PR.
- The next topic is the `0.1.0-beta.5` release. After it comes the `--db` with a URL target bug,
  the only open Severity-1 item in `docs/UI_TASKS.md`.
- The tag is pushed only after the user says so.

## Shared rules

- Use the glossary's words, from `CONTEXT.md`. Be accurate before complete.
- Plan docs are history. Update their **Status** lines and add dated notes. Don't rewrite what
  they decided.
- Generated references (`site/content/docs/reference/{keybindings,cli}.md`) are never hand-edited.
- Never remove Docker containers you didn't create.
- No tags, and no repo-settings changes.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. PR bodies end with
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Edit PR bodies with `gh api`,
  because `gh pr edit` can fail.

## Task 1: doc fix-up

Branch `docs-verify`, one PR. Every item was verified against `231e2d5`.

1. **`CLAUDE.md`**
   - The "Project state" header says "Scaffolded (M0.1 complete)… there is no UI yet". Replace
     it with the real state in two or three lines: M0–M6 done and released through
     `0.1.0-beta.4`, with `docs/PLAN.md` holding the per-milestone progress. Keep the bullet list
     of docs under it.
   - Remove "Once `Cargo.toml` exists, fill in the commands below."
   - "only dedups in-flight fetches only" → "only dedups in-flight fetches".
   - "109 tests in `crates/app/tests/integration.rs`": the file has about 200 `#[ignore]`d tests
     now. Drop the number ("The integration suite **exists** in
     `crates/app/tests/integration.rs`, every test `#[ignore]`d…") so it can't go stale again.
2. **`site/content/docs/contributing.md`**, "Working on this book": it still says `book/src`,
   `cargo install mdbook` and `mdbook serve book`.
   - Rename the section to "Working on this guide".
   - Pages are plain `.md` in `site/content/docs`, ordered by each folder's `meta.json`. Preview
     with `pnpm --dir site install` then `pnpm --dir site dev` (http://localhost:3000/redis-pane).
     Node is pinned in `site/.nvmrc`, and pnpm by `site/package.json`.
   - Keep the generated-pages part and the `UPDATE_DOCS=1` commands. Mention that ```` ```json ````
     blocks are parsed by the real config parser.
3. **`docs/UI_TASKS.md:22`**: "the book (Databases)" → "the user guide (Databases)". Check that
   the user guide's Databases section really documents the `--db` bug, and fix the pointer if it
   has moved.
4. **`docs/PLAN.md`**
   - Header: the title "Implementation plan, M0 and M1", "Draft v0.1" and "Last updated
     2026-08-26" are stale. Retitle it to "Implementation plan" (M0–M6 and after), set Last updated to
     2026-10-20, and update the intro paragraph so it doesn't claim to cover only two milestones.
   - Add a section after §7.3 (before §8): **"7.4 After M6: docs, site, org"**. It is a short
     table of the work in #93–#114, each row linking its plan doc: `docs-planning.md` and
     `docs-task1..3`; `org-followups-planning.md` and `org-task1..2`; `site-fumadocs.md`;
     `demos.md`; and the brand (#113, #114, no plan doc). End it with: "Unreleased until
     `0.1.0-beta.5`."
   - Add a line for the M2 tail's `beta.4` release if §5 doesn't already name it.
5. **Stale plan statuses.** Set each one to done, with its PRs:
   - `plans/docs-planning.md` ("planned"): done in #94–#96. Add that the mdBook site was
     superseded by Fumadocs (#102).
   - `plans/site-fumadocs.md` ("planned"): done in #102 and #103.
   - `plans/org-followups-planning.md` ("planned; Settings done"): tasks 1 and 2 done in #100
     and #101. Add a dated note that the `mdbook build · link check` job is now `site build · link
     check`, and still isn't required.
   - Read `docs-task1..3`, `org-task1..2` and `demos.md`, and fix any Status line that isn't done.
6. **PRD §10, packaging**: the decision was "scheduled for the end of alpha", and alpha has
   ended. Reword it as **still open, not scheduled**, and note that the betas ship the same
   unsigned `cargo-dist` archives. Do not decide it.
7. **Sweep** for anything else this misses:
   - `grep -rnIi "mdbook\|book/src\|the book\b" README.md TESTING.md CLAUDE.md CONTEXT.md docs/*.md docs/adr site/content .github scripts tapes`.
     Anything outside historical plan docs and `CHANGELOG.md` gets fixed.
   - Check that README links and the docs site's internal links still resolve. `pnpm --dir site
     build` link-checks the site.
   - Fix a small, sure mismatch in place. Report anything bigger in the PR body instead of fixing
     it.

**Checks:** `cargo test --workspace` (it includes both `docs_reference` tests), `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `pnpm --dir site
build`.

## Task 2: release `0.1.0-beta.5`

Branch `release-beta5`, one PR. Follow [`../RELEASING.md`](../RELEASING.md) steps 2–6 exactly.

- Bump `[workspace.package] version` to `0.1.0-beta.5`, and let `Cargo.lock` follow
  (`cargo build`).
- Re-pin the installer URLs and versions in:
  - `README.md`, including the Compatibility section
  - `TESTING.md`, including its first paragraph
  - `site/content/docs/getting-started/installation.md`
  - `site/lib/release.ts`
- The asset-size footnote (README `[^2]` and `site/content/docs/index.mdx`) names the `beta.4`
  asset. Leave it naming `beta.4` explicitly, because it's a measurement of that release. It is
  re-measured after `beta.5`'s assets exist.
- `CHANGELOG.md`:
  - Move `Unreleased` under `## [0.1.0-beta.5] - 2026-10-20`, with a fresh empty `Unreleased`.
  - Add the user-visible items from #93–#114 that aren't there yet: the new docs site, the
    landing page, the demos, and the org move with its new install URLs. Keep it user-facing.
    Repo-internal items (CI, issue forms) go in at most one line.
  - Fix the compare links at the bottom.
- `grep -rn "0.1.0-beta.4"`: only the asset-size footnote and `CHANGELOG.md` history remain.
- Mark this plan's Status done for both tasks.

After merge (main session, not the subagent):
1. Ask the user before tagging the merge commit `v0.1.0-beta.5` and pushing it.
2. Verify the assets.
3. Work through RELEASING's "First release from the org" checklist.
