# Launch groundwork (promotion phase A)

Status: **done**. Planned 2026-10-23. One task, one Sonnet subagent, one PR. It merges once every check
has finished and passed.

## Context

`1.0.0` shipped on 2026-10-22, but the project is effectively unknown: 2 stars, and 2 unique
visitors in 14 days. The user is preparing to promote it, through a soft launch on r/rust,
Ratatui and This Week in Rust, then Show HN. A gap audit on 2026-10-23 found what makes a shared
link or a first visit fall flat:

- **No social card.** Neither GitHub nor the docs site has an Open Graph image, so links show a
  generic card.
- **No `sitemap.xml` or `robots.txt`** on the site.
- **A 37% GitHub community profile:** no `CONTRIBUTING.md` (the README sends contributors to
  `CLAUDE.md`), code of conduct, `SECURITY.md` or PR template.
- **Windows advertised as supported** while #138 documents that Profiles, session restore and
  `passwordCommand` don't work there.
- **A beta-era release badge.**

This task fixes everything in that list that lives in the repo. Settings-only items are the
user's: uploading the social preview, enabling private vulnerability reporting, and the trademark
check.

## Deliverables

### 1. Social image and site metadata

- **One social image, 1280×640 PNG,** in the brand's style: the `>:_` lockup from
  `site/public/brand/`, the one-line pitch, and a frame of the hero demo
  (`site/public/demos/hero.png`). Two acceptable routes:
  - **Preferred:** a Next `app/opengraph-image.tsx` (`ImageResponse`), if it renders at build
    time under `output: 'export'` with the `/redis-pane` base path. Check the built `site/out`.
  - **Otherwise:** a checked-in PNG at `site/public/brand/social.png`, produced by a small
    reproducible script (e.g. `scripts/` plus `rsvg-convert` or ImageMagick, if installed).
    Commit the script.

  Either way, write the final PNG to `/Users/vinodsantharam/.claude/jobs/77c4d745/tmp/social-preview.png`
  too, for the user to upload as the GitHub social preview.
- **`site/app/layout.tsx` metadata:**
  - `metadataBase` = `https://redis-pane.github.io/redis-pane/`;
  - `openGraph` (title, description, url, siteName, image 1280×640, type `website`);
  - `twitter` (`summary_large_image`, the same image);
  - a per-page title template if Fumadocs doesn't already set one.
- **One description everywhere.** `site/lib/shared.ts` `siteDescription` should say the same thing
  as the GitHub repo description: "A terminal UI for Redis: browse a keyspace, watch keys update
  live, and edit safely, over SSH too." Mention Valkey if it fits naturally. Use it in the
  metadata.
- **`site/app/sitemap.ts` and `site/app/robots.ts`,** working under the static export, with
  absolute URLs under the base path. The sitemap lists the landing page and every docs page;
  Fumadocs' `source.getPages()` can enumerate them.

### 2. Community files

- **`CONTRIBUTING.md`, written for a human.** It covers:
  - prerequisites (Rust via `rust-toolchain.toml`; Node and pnpm for the site);
  - build, run and test, including the two suites (default and Docker integration) and when each
    is needed;
  - the generated docs (`UPDATE_DOCS=1`);
  - the core/shell boundary rule in one paragraph;
  - the PR process (one change per PR, docs updated in the same PR, CI must be green);
  - where decisions live (`CONTEXT.md`, `docs/adr/`, `docs/PRD.md`, `docs/DESIGN.md`);
  - "issues labelled `good first issue`".

  Take the facts from `CLAUDE.md`, but don't send humans to `CLAUDE.md`. Mention it only as the
  agent-facing guide.
- **`CODE_OF_CONDUCT.md`:** Contributor Covenant 2.1, with the enforcement contact as the
  repository's private reporting channel or the maintainer's GitHub profile. **Do not invent an
  email address.**
- **`SECURITY.md`:**
  - report vulnerabilities through GitHub's private vulnerability reporting (the "Report a
    vulnerability" button), not public issues;
  - supported versions: the latest release;
  - what's in scope: credential handling, config parsing, the mutation safety rails.
- **`.github/pull_request_template.md`:** a short template with what and why, linked issue,
  requirement IDs if any, docs updated, and tests.
- **README "Feedback, contributing and license":** point contributors to `CONTRIBUTING.md`, and
  stop sending them to `CLAUDE.md`. The user guide's `site/content/docs/contributing.md` gets the
  same change.

### 3. README first screen and badges

- The release badge drops `?include_prereleases`.
- Right after the hero GIF's caption, add an install line: `brew install redis-pane/tap/redis-pane`,
  with "other platforms" linking to the Install section.
- The tagline names Valkey, e.g. "A terminal UI for Redis and Valkey: …". Keep it one line.

### 4. Windows marked experimental (until #138 is fixed)

- **README Compatibility and Install:** Windows is **experimental**. Link #138 and say briefly what
  doesn't work yet: Profiles and session restore need `HOME` set, `passwordCommand` needs `sh`.
- **`site/content/docs/limits.md`:** a Windows item with the same facts and the link. The
  installation page's Windows lines say "experimental" too.
- No code changes.

### 5. Labels

- `gh issue edit 112 --add-label "good first issue"`.
- Don't relabel anything else.

### 6. Docs

- `CHANGELOG.md` `Unreleased`: one line (community files, Windows marked experimental). Site
  metadata isn't user-facing; leave it out.
- This plan: Status set to done.

## Shared rules

- Use the glossary's words, from `CONTEXT.md`.
- Don't hand-edit the generated reference pages.
- No repo-settings changes and no tags. The social preview upload and private vulnerability
  reporting are the user's.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The PR body
  ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Pass it with
  `--body-file`, and edit it only through `gh api`.

## Checks

- `pnpm --dir site build`, then in `site/out`:
  - `index.html` has `og:image`, `og:title`, `og:description` and `twitter:card`, all absolute URLs
    under `https://redis-pane.github.io/redis-pane/`;
  - the image file exists at that path;
  - `sitemap.xml` and `robots.txt` exist and use the base path.
- `cargo test --workspace`, clippy and fmt (no Rust changes are expected, but CI runs them).
- After merge (main session): `gh api repos/redis-pane/redis-pane/community/profile --jq .health_percentage`.
