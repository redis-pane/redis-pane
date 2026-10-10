# Docs site: mdBook → Fumadocs (Next.js), static on GitHub Pages

Status: **planned** 2026-10-10. Two tasks. Each runs on one Sonnet subagent, and each PR merges
once every check passes.

## Context

The user guide is an mdBook site (`book/`, 37 chapters) at https://redis-pane.github.io/redis-pane/.
The user finds its look dated and wants a modern site in the Vercel / Next.js style.

## Decisions (user, 2026-10-10)

- **Fumadocs** on Next.js, chosen over Nextra 4, Starlight, Docusaurus and Mintlify. It has the
  Vercel look (sidebar, TOC, ⌘K search, dark mode), it's real Next.js and React, and it renders
  plain `.md`, so the Rust-generated pages keep working.
- **Hosting stays on GitHub Pages at the same URL.** Next.js runs with `output: 'export'` and
  `basePath: '/redis-pane'`. There's no Vercel account and no new service.
- **A landing page at `/`** (hero, demo, comparison, features, install), with the guide under
  `/docs`.

## Shared rules

- **The generated pages stay generated.**
  - `crates/core/tests/docs_reference.rs` and `crates/app/tests/docs_reference.rs` write
    `reference/keybindings.md` and `reference/cli.md`, and fail CI when those are stale.
  - The core test also parses every ```` ```json ```` block in the docs with the real config
    parser.
  - Repoint them to the new content directory and keep `UPDATE_DOCS=1` regeneration. Never
    hand-edit a generated page.
  - If Fumadocs needs frontmatter, the generator emits it.
  - Keep the generated pages as `.md`, not MDX, so `<`, `{` and `|` need no escaping.
- **Content is migrated, not rewritten.** Use `git mv` so history follows each page. Change only
  what the new framework needs: links, frontmatter, and mdBook-only syntax. The words were
  reviewed in #95, #96 and #100.
- **Pin the toolchain:**
  - pnpm, with the `packageManager` field in `package.json`;
  - Node LTS in `.nvmrc`;
  - a checked-in lockfile;
  - exact Fumadocs and Next.js versions recorded in the PR.
- **Colors and theming:** use Fumadocs' theme tokens and Tailwind. Dark mode works, and nothing
  is hard-coded per page.
- **No tags, no repo settings changes.** Never remove Docker containers you didn't create
  (`redis-pane-dev`, `redis-pane-cluster`).
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. PR bodies end with
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. `gh pr edit` can fail with
  a Projects-classic error; use `gh api` to edit a PR body.

## Task 1: scaffold, migrate, CI

1. **Scaffold `site/`.** Use the current Fumadocs Next.js template (`create-fumadocs-app`, or
   follow its manual setup) with:
   - `output: 'export'`, `basePath: '/redis-pane'` and `images: { unoptimized: true }`;
   - `trailingSlash: true` if it makes GitHub Pages serve `/docs/x/` cleanly;
   - static search: Fumadocs' static Orama mode, a pre-rendered search route plus
     `useStaticSearch` (or the current equivalent);
   - the site title "redis-pane", the repo link `https://github.com/redis-pane/redis-pane`, and a
     favicon.
2. **Migrate the content.** `git mv book/src/<chapter>` → `site/content/docs/<chapter>`. Then:
   - Turn `SUMMARY.md`'s order and sections into Fumadocs `meta.json` files: the same sections,
     the same order.
   - Add `title` (and `description`, where obvious) frontmatter. The first `# H1` becomes the
     title.
   - Rewrite internal links from `.md`/`.html` to Fumadocs routes. Update anchors if the slug
     rules differ.
   - Move images to `site/public/images/` and fix their paths for `basePath`.
   - The book's `README.md` (Introduction) becomes `docs/index.md`.
3. **A minimal landing page** at `/`: a title, the one-line pitch, the demo GIF, and "Read the
   docs" and GitHub buttons. Task 2 builds the full page.
4. **Generators and tests.** Repoint both `docs_reference.rs` tests and the JSON-example scan to
   `site/content/docs`. Regenerate with `UPDATE_DOCS=1`. Check that a deliberate drift still
   fails, then revert it.
5. **CI (`.github/workflows/docs.yml`):**
   - Path filters change to `site/**`.
   - Set up pnpm and Node from `.nvmrc` with a cache, run `pnpm install --frozen-lockfile`, then
     `pnpm build` (static export to `site/out`).
   - Run lychee offline over `site/out`, excluding the 404 page.
   - Upload `site/out` as the Pages artifact.
   - Keep the job names unless a rename is needed. The check isn't required by branch
     protection, so a rename is safe.
6. **References:**
   - CLAUDE.md commands: `pnpm --dir site install`, `pnpm --dir site dev` (preview at
     http://localhost:3000/redis-pane), and the regeneration commands with their new paths.
   - README docs links: new URLs under `https://redis-pane.github.io/redis-pane/docs/...`.
   - `docs/RELEASING.md`: the installation page path.
   - TESTING.md links.
   - `.gitignore`: `site/node_modules`, `site/.next`, `site/out`, `site/.source`.
   - Check the old plan docs only for live instructions; leave history alone.
7. **Remove `book/`** (`book.toml` and the leftovers), and remove `mdbook` from CI.

**Testing:**
- `pnpm --dir site build` with no errors or warnings that matter, and lychee clean over
  `site/out`.
- `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace` all pass.
- Serve `site/out` under `/redis-pane` locally, for example by copying it into a temp dir as
  `redis-pane/` and running `python3 -m http.server`. Click through:
  - the landing page;
  - `/docs`;
  - a deep page;
  - `reference/keybindings`;
  - search, which returns results;
  - dark mode;
  - a narrow width.

  Include a screenshot or a short description in the PR.

## Task 2: the landing page

Build `/` as a Vercel-style product page in React and Tailwind, inside the Fumadocs layout (its
navbar and theme):

- **Hero:** the name, the one-line pitch, and the install command in a copyable code block (the
  shell installer, with a tab for PowerShell). Buttons for "Get started" (`/docs`) and GitHub.
- **Demo:** `demo.gif`, framed like a terminal window.
- **Feature grid:** the README's six groups (Browse, See it live, Edit safely, Watch the server,
  Cluster & Sentinel, Fits any terminal). Each card has an icon (`lucide-react`, which Fumadocs
  already uses), two lines of text, and a link into the docs.
- **The comparison table:** the README's, with its footnotes, kept identical in content.
- **"What it doesn't do":** short, linking to the Limits page.
- **Footer:** docs, releases, Discussions and license.
- Everything is static. No analytics, no external fonts beyond what Fumadocs ships, and no
  tracking.
- The installer URL and version come from **one constant** in the site code. Add that file to
  `docs/RELEASING.md`'s bump list.

**Testing:** a clean build, lychee clean, and checks at desktop and mobile width, in light and
dark. Include screenshots or a description in the PR.
