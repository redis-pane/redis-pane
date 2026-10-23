# Launch content (promotion phase B, repo side)

Status: **done** 2026-10-23. One task, one Sonnet subagent, one PR. It merges once every check
has finished and passed.

## Context

The project's groundwork for promotion is done (#139: social card, site metadata, community files;
#140: the Redis trademark notice). The maintainer has published the launch article on a personal
site: ["The Refresh Button That Didn't Refresh"](https://vinodsantharam.github.io/blog/the-refresh-button-that-didnt-refresh/)
(2026-10-22). It tells the origin story: RedisInsight's refresh button didn't update the open
value, and that led to a TUI whose open key updates live.

This task does the repo side of phase B. The user chose items 2–4 of the revised plan:

1. **Link the article from the project,** so visitors get the story behind it.
2. **A "What's next" page in the user guide,** so visitors see the project is alive and where
   it's going.
3. **A 20–30 second clip for social posts,** made with the existing `tapes/` pipeline.

The article itself, the optional second article and the launch-post drafts are the maintainer's;
they stay out of this repo.

## Deliverables

### 1. Link the article

- **README, "Why" section:** after its first paragraph, add one line: "The story behind it: [The
  Refresh Button That Didn't Refresh](https://vinodsantharam.github.io/blog/the-refresh-button-that-didnt-refresh/)."
  Keep the comparison table where it is.
- **Landing page (`site/app/(home)/page.tsx`), its "Why" section:** the same link, in the page's
  existing link style. Put the URL in one place (`site/lib/release.ts` `links`, or
  `site/lib/shared.ts`) and reuse it.
- **User guide introduction (`site/content/docs/index.mdx`):** one sentence with the link, under
  "Who it is for".
- Describe it as the story behind the project. Don't call it documentation, and don't repeat its
  claims about RedisInsight.

### 2. "What's next" page

- **`site/content/docs/whats-next.md`,** added to `site/content/docs/meta.json` between
  `troubleshooting` and `limits`. Title "What's next", with a one-line description.
- **Content,** built from the open GitHub issues, each one sentence plus its link. Read each issue
  (`gh api repos/redis-pane/redis-pane/issues/N --jq .body`) and describe it accurately:
  - **#138**, Windows: verify the build and installer, and fix the config, state and
    `passwordCommand` paths. The highest priority, because Windows is marked experimental until
    then;
  - **#122**, copy the selected nested JSON value;
  - **#110**, keybinding overrides: a decision is pending;
  - **#109**, the fuzzy filter: a decision is pending;
  - **#112**, finer bulk-delete progress (labelled `good first issue`).
- **Also link:**
  - the Limits page, for what is deliberately not built;
  - the open issues list;
  - Discussions, for ideas;
  - `CONTRIBUTING.md`.
- **No dates and no promises.** Say the order may change, and that the issue tracker is the
  source of truth.
- **The landing page** links to "What's next", e.g. in its footer nav next to Docs/Releases.

### 3. Social clip

- **A new tape, `tapes/social.tape`,** using `_settings.tape` and `render.sh` like the others, with
  its seeding in `render.sh`'s `seed()` if needed. One take, 20–30 seconds:
  1. open a key while another client changes it (the `hero` and `live` mechanics, with churn
     `--focus`), so the value visibly updates with no keypress;
  2. then a safe edit: edit a value, so the command preview appears, then confirm.

  Reuse the existing tapes' mechanics (`hero.tape`, `live.tape`, `edit-value.tape`). Don't invent
  new ones.
- **Output:** `render.sh` writes `site/public/demos/social.{mp4,webm,png}`. No GIF.
- **Budget:** `tapes/README.md` sets 1 MB per video for 8–20 s. This clip is longer and is meant
  for social posts, not the site. Allow it up to **3 MB** and record that exception in
  `tapes/README.md`, with the reason. Social platforms autoplay MP4, so make sure the `.mp4`
  plays: H.264, `yuv420p`.
- **It isn't embedded on any page.** It lives at a stable URL
  (`https://redis-pane.github.io/redis-pane/demos/social.mp4`) for the maintainer to download and
  post.
- **Follow every `render.sh` rule** (`tapes/README.md`: its own container `redis-pane-demo` on
  :6390, temp XDG dirs, never touch `redis-pane-dev` or `redis-pane-cluster`). Render only this
  tape (`./tapes/render.sh social`), not the others.
- **Watch the result:** extract a few frames with `ffmpeg` and view them. Confirm the live update
  and the preview are visible and legible.

### 4. Docs

- `CHANGELOG.md` `Unreleased`: one line for the "What's next" page.
- This plan: set Status to done.

## Shared rules

- Use the glossary's words, from `CONTEXT.md`.
- The trademark notice from #140 must appear on the new docs page. It renders automatically; check
  it in `site/out`.
- Don't hand-edit the generated reference pages.
- No repo-settings changes, no tags, and no issue or label changes.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. The PR body
  ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Pass it with
  `--body-file`, and edit it only through `gh api`.

## Checks

- `pnpm --dir site build`:
  - `site/out/docs/whats-next/index.html` exists and carries the trademark notice;
  - the article link appears in `site/out/index.html` and `site/out/docs/index.html`.
- `cargo test --workspace`, clippy and fmt (CI runs them; no Rust changes are expected).
- `ffprobe site/public/demos/social.mp4`: duration 20–30 s, H.264, `yuv420p`, at most 3 MB.
- After rendering, `docker ps -a --filter name=redis-pane-demo` is empty.
