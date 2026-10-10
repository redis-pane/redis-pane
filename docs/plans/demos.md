# Demos: tapes, one copy of each recording, Tier 1 scenarios

Status: **planned** 2026-10-10. Two tasks. Each runs on one Sonnet subagent, and each PR merges
once every check passes.

## Context

- **The GIFs are committed twice.** `demo.gif`, `navigation-demo.gif` and `monitor-demo.gif`
  (4.4 MB) are in the repo root and again in `site/public/images/`.
- **The tapes sit with the dev tools** in `scripts/`, and they're stale. `demo.tape` presses
  `yc`, but copying a `redis-cli` command is now `C`, and they cite `book/src/...` paths.
- **The hero GIF opens on an almost empty terminal,** so the landing page shows a dark box for
  its first second.
- **Most docs pages have no recording at all.**

## Decisions (user, 2026-10-10)

- **Layout, following the usual VHS and Next.js conventions:**
  - **`tapes/`** holds the tape sources: one `.tape` per scenario, plus `_settings.tape` (shared
    `Set` lines, pulled in with `Source`), `render.sh` and a `README.md`.
  - **`site/public/demos/`** is the only copy of the rendered output. For each scenario:
    - `<name>.webm` and `<name>.mp4`, which the site plays as a looping video;
    - `<name>.png`, the poster image, taken from a frame mid-session;
    - `<name>.gif`, only for scenarios the README embeds.
  - The README links to `site/public/demos/<name>.gif` by relative path. The GIFs in the repo
    root and in `site/public/images/` are deleted.
- **Scope is Tier 1 only:** scenarios 1–13 below. Tier 2 waits.
- **Recordings are rendered locally by `tapes/render.sh`, not in CI.** The rendered files are
  committed.

## Shared rules

- **Nothing from your own machine leaks into a recording.** `render.sh` must:
  - start its **own** container, `redis-pane-demo`, on port **6390** (`redis:8.4-alpine`), and
    remove only that container when it finishes;
  - **never touch `redis-pane-dev` or `redis-pane-cluster`**, which belong to the user;
  - seed the data deterministically, with `scripts/fixtures.py --seed` plus small seed steps per
    scenario;
  - set `XDG_CONFIG_HOME` and `XDG_STATE_HOME` to temp dirs, so no real Profile or saved session
    is used. Scenarios that need a Profile, such as a `prod` one, write it into that temp config
    with `chmod 600`;
  - build `target/release/redis-pane` first and put it first on `PATH`.
- **Every tape uses `--url redis://127.0.0.1:6390`**, or a Profile name from the temp config, so
  connection resolution can't pick anything else up.
- **Keys come from the generated reference** `site/content/docs/reference/keybindings.md`, not
  from memory. A tape whose key doesn't do what its comment says is a bug in the tape.
- **Shared look, from `_settings.tape`:**
  - the theme matches the site's dark look (keep Dracula unless something else clearly suits the
    site better);
  - font size 18, width 1200, height 720, padding 20;
  - one typing speed for every tape.
- **Size budget:** a `.webm` or `.mp4` is at most 1 MB, and a `.gif` at most 1.5 MB. Use
  `Set Framerate` or shorter scenarios to stay under. Each scenario is 8–20 s.
- **Hidden setup:** background writers (`churn.py`, publishers) start under `Hide` and are
  killed at the end of the tape.
- **Site playback:** a single `Demo` React component in `site/components/`, written as
  `<video autoPlay muted loop playsInline preload="metadata" poster=…>` with webm and mp4
  sources. Its paths include `basePath`. Pages use it in MDX. A page that embeds a demo becomes
  `.mdx`; the generated reference pages stay `.md`.
- No tags and no repo settings changes. Commits and PR bodies end with the usual attribution
  lines. Edit PR bodies with `gh api`, because `gh pr edit` can fail.

## Tier 1 scenarios

| # | Tape | Shows | Embedded in |
|---|---|---|---|
| 1 | `hero` | Filter to a key, open it, and watch it change with no keypress (churn `--focus`). The poster frame is mid-update | Landing page, the docs `index`, README (GIF) |
| 2 | `browse` | Tree folding with `←`/`→`, `t` to flat, `s` cycling sorts, type/memory/TTL filling in | `browsing/key-list`, README (GIF) |
| 3 | `filter` | A substring filter, then a glob (`user:*:session`), then `Esc` | `browsing/filter-sort` |
| 4 | `types` | String with JSON, Hash, List, Set, Sorted set, Stream, and a binary value in hex | `viewing/types` |
| 5 | `live` | A value changing live, a TTL counting down, and the key deleted from another client, which shows the deleted badge | `viewing/live-updates` |
| 6 | `edit-value` | Edit a JSON String inline, `⌃S`, the command preview, `y` | `editing/values` |
| 7 | `edit-members` | Add a Hash field, change a Sorted-set score, remove a List element, each through its preview | `editing/values` |
| 8 | `safety` | A `prod` Profile opens read-only, `d` shows the preview, `y` is refused, `?` shows the dimmed keys, `⌃R` unlocks | `safety/read-only`, `safety/previews` |
| 9 | `bulk-delete` | `Space` marks, `d`, the preview, the typed count on `prod`, the `deleting X of N` progress | `editing/bulk-delete` |
| 10 | `dashboard` | `g d`, the tiles and ops/sec moving under churn, `Enter` for the raw `INFO` | `server/dashboard` |
| 11 | `monitor` | `g m`, a live tail, `p` to pause, `/` to filter. Replaces `monitor-demo` | `server/monitor`, README (GIF) |
| 12 | `pubsub` | Subscribe to a channel and a pattern, with messages from a hidden publisher loop | `server/pubsub` |
| 13 | `slowlog` | `g s` and sorting. The demo Redis gets `CONFIG SET slowlog-log-slower-than 0`; never the user's Redis | `server/slowlog` |

## Task 1: layout, render pipeline, hero

1. **Create the files:**
   - `tapes/README.md`: prerequisites (`brew install vhs`, Docker, Python 3) and usage
     (`./tapes/render.sh` for all, `./tapes/render.sh hero` for one);
   - `tapes/_settings.tape`;
   - `tapes/render.sh`, following the shared rules. It is idempotent, cleans up on exit with a
     trap, and fails loudly.
2. **Move and rename tapes** with `git mv`:
   - `scripts/demo.tape` → `tapes/hero.tape`
   - `scripts/navigation-demo.tape` → `tapes/browse.tape`
   - `scripts/monitor-demo.tape` → `tapes/monitor.tape`

   Fix the stale keys (`C`, not `yc`) and paths, switch to `Source _settings.tape`, and set the
   output to `site/public/demos/`. Update `scripts/README.md`.
3. **Render all three** to webm, mp4 and a png poster, plus a GIF for the README ones (hero,
   browse, monitor). Pick a mid-session poster frame, either with VHS `Screenshot` or with
   ffmpeg (`brew install ffmpeg` if it's missing).
4. **Add the `Demo` component.**
   - The landing page's hero and the docs pages that embed the old GIFs (`index`,
     `getting-started/quick-start`, `browsing/key-list`, `server/monitor`) switch to `Demo`.
   - The README uses the GIFs at `site/public/demos/`.
5. **Delete** `demo.gif`, `navigation-demo.gif` and `monitor-demo.gif` from the root, and
   `site/public/images/*.gif`. `grep` for any leftover reference.
6. **Update CLAUDE.md:** a "Recording demos" line in Commands, and remove the old VHS mention if
   there is one.

**Testing:**
- `pnpm --dir site build` and lychee are clean, and `cargo test --workspace` passes.
- Serve the export under `/redis-pane`. Take a headless Chrome screenshot of the landing page
  showing the poster, not a dark box, plus one docs page with a demo.
- Report each file's size against the budget.
- `docker ps` afterwards shows `redis-pane-dev` untouched and no `redis-pane-demo` left
  running.

## Task 2: the rest of Tier 1

- Write and render tapes 3–10, 12 and 13. Each needs its seed steps in `render.sh` and is
  embedded in its page with `Demo`.
- Render every Tier 1 tape once more with the final settings, so the whole set matches.
- Run the same checks as task 1, and put a size table for all 13 scenarios in the PR.
