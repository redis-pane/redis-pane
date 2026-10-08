# Docs task 3: README and CHANGELOG

Part of [`docs-planning.md`](docs-planning.md). Read its shared rules first. Builds on task 2's
book.

## Context

The README is the front door, and today it is a release log. It should answer three questions in
a minute: what is this, what can it do, and what can't it do. Everything deeper is in the book.

## Deliverables

1. **README rewritten** to about 150 lines, in this order:
   1. **Title, pitch and demo.** The title, a one-line pitch, and badges for CI, the latest
      release (including prereleases) and the license. Then `demo.gif` and its caption.
   2. **Why.** Three sentences, then the comparison table:
      - Rows:
        - runs in a terminal and over SSH;
        - single binary with its size, taken from the beta.4 release asset;
        - the open key updates live, by push with no refresh;
        - a million keys browsable, measured, citing the perf harness;
        - the exact command is shown before every write;
        - writes are locked by Environment;
        - Cluster;
        - Sentinel;
        - mouse optional.
      - Columns: redis-pane, `redis-cli`, RedisInsight.
      - **Every competitor cell is verified**: use RedisInsight's own docs and README, and
        `redis-cli --help`. Use only "✓ / ✗ / partial" with a footnote where it is partial.
      - List the source for each competitor cell in the PR body.
      - No performance numbers for other tools.
   3. **Features**, grouped. Each group has 3–5 bullets and ends with a "→ docs" link into the
      book chapter:
      - Browse
      - See it live
      - Edit safely
      - Watch the server
      - Cluster & Sentinel
      - Fits any terminal (themes, ASCII, 80×24, mouse, SSH clipboard)
   4. **What it doesn't do:** the book's Limits page condensed into one honest list.
   5. **Install:** today's section, kept as is.
   6. **Quick start:**
      - connect with no arguments, with `--url`, and with a Profile name;
      - the handful of keys that matter;
      - "`?` shows every key that works where you are", plus a link to the keybindings
        reference.
      - Keep `navigation-demo.gif` here.
   7. **Documentation:** the site URL and its main chapters.
   8. **Compatibility:**
      - beta;
      - Redis 6.0+ and Valkey;
      - standalone, Sentinel and Cluster;
      - TLS;
      - prebuilt for macOS (arm64 and x86_64), Linux x86_64 and Windows;
      - build from source for anything else.
   9. **Feedback, contributing and license:** `TESTING.md`; `CLAUDE.md`, `CONTEXT.md` and
      `docs/`; and the license.

   Keep `monitor-demo.gif` in Features or drop it, whichever reads better. Don't add new GIFs.
2. **The book's Introduction table** is made identical to the README's.
3. **`CHANGELOG.md`** in [Keep a Changelog](https://keepachangelog.com) style, newest first:
   - an `Unreleased` entry (the `unknown` typed count, and these docs);
   - then `0.1.0-beta.4`, `beta.3`, `beta.2`, `beta.1`;
   - the alphas summarised in a single entry.

   Sources are `gh release view <tag>` and the PR titles between tags (`git log --oneline
   vA..vB`). Describe each change in user terms.
4. **The release procedure.** The close-out plan docs (`m6-close-out.md` is the latest) and
   anywhere else that lists release steps now include:
   - bump the installer URLs in README, `TESTING.md` and the book's installation page;
   - move `Unreleased` in CHANGELOG under the new version.

   If there is no single release checklist, create `docs/RELEASING.md` and link it from
   CLAUDE.md.

## Testing

- Every link in the README resolves. Check this with lychee or by hand, and include the book
  URLs once the site is live.
- `cargo test --workspace` still passes.
- Proofread the README rendered (`gh markdown-preview` or a GitHub preview on the PR).

## Out of scope

Product changes, new screenshots or GIFs, and a version bump.
