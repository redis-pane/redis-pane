# Changelog

All notable changes to `redis-pane` are listed here, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The project is a beta and does not yet
promise a stable interface.

## [Unreleased]

### Added

- `o` in the key list moves the cursor back to the Open key. If a filter hides it, `o` clears the
  filter and says so; if a collapsed group hides it, `o` unfolds the groups above it. With no key
  open it says `no key is open`. `?` lists it as `go to open key` while a key is open.
- `←` (or `h`) in the value pane now takes you back to the key list, the way `→` takes you in. It
  does what `Esc` does there: focus returns to the key list and the value cursor turns off, with
  the key staying open. It works at every width, including the single-pane layout below 70
  columns, and `?` lists it as `back to keys`.

### Fixed

- With the value pane focused and the value cursor off, `←` collapsed or climbed the tree group
  under the key list's cursor behind your back, so the open key stopped matching the selected one.
  It now leaves the value pane and does not touch the key list.

## [0.1.0-beta.6] - 2026-10-21

### Fixed

- `--db` now applies when the target is a URL (`--url`, `REDIS_URL` or a Profile's `url`). It
  replaces the URL's own database in what is dialled, and the title bar, `--print-target` and
  `--probe` show that database. Before, the connection used the URL's database while the title bar
  showed `--db`, and a URL that named one displayed as `/2/3`. The order is `--db`, then the
  Profile's `db`, then the URL's, then 0. A Cluster accepts only database 0, and a
  `redis-cluster://` URL with any other is refused at startup with exit code `3` (#106).
- With Read-only Mode on, the title bar no longer cuts the target to `…90/0` at about 100 columns.
  When the bar is too narrow it now drops the liveness readout before it shortens the target, and
  keeps the READ-ONLY badge and a dropped-link warning (`✕ disconnected`) longest (#107).
- In tree view, `s` now says why it does nothing. Pressing it shows
  `tree view is sorted by name · t for flat view`, the hint bar no longer offers `s`, and `?`
  shows it dimmed as `sorted by name in tree view`. Before, the key was advertised and silently did
  nothing (#108).
- At 80 columns, the `?` help overlay's line of global keys no longer cuts off `quit` and `help`.
  It wraps onto a second line instead (#111).

## [0.1.0-beta.5] - 2026-10-20

### Changed

- A Profile named on the command line that the config file doesn't define, by `--profile` or as a
  bare name, is now an error that names it and lists the available Profiles. It exits with code
  `3`. Before, it silently fell back to `127.0.0.1:6379`.
- The help overlay lists `Enter` (open into the value), `←`/`h` (collapse or parent), `⌃←`/`⌃→`
  (resize the panes) and `⌃Y` (redo, in the editor), and shows `⌃C` beside `q`. `Enter` is spelled
  `⏎` everywhere.
- `redis-pane --help` describes `--host` and `--port`, explains how the target is chosen, and no
  longer cites an internal document under `--db`.
- The keybindings reference no longer says you can override bindings. Keys can't be rebound.
- Bulk delete on an `unknown` Environment now asks you to type the count of keys, as it already
  did on `prod`.

### Added

- A user guide as a website at <https://redis-pane.github.io/redis-pane>, built from `site/`
  (Fumadocs on Next.js), with a landing page and thirteen recorded demos, from browsing and live updates to
  editing, safety and the server views.
- The project now lives at `redis-pane/redis-pane`. The installer URLs, the release downloads and
  the docs links point there; the old `vinodsantharam/redis-pane` URLs redirect.
- The `>:_` icon and the `redis:pane` wordmark.
- Issue forms and community files for the repository.
- Keybinding and command-line references generated from the code, so they cannot drift from the
  program.
- A comparison table in the README, and this changelog.
- `docs/RELEASING.md`, the release checklist.
- `TESTING.md` replaces `ALPHA.md` as the "what to try in this beta" page.

## [0.1.0-beta.4] - 2026-10-08

### Added

- Rename a key (`R`), duplicate it with `COPY` (`D`), and rename a Hash field or a Set or Sorted set
  member in place. Nothing overwrites an existing name.
- Mark keys with `Space` and delete the marked set in bulk. On `prod` you type the count to confirm.

## [0.1.0-beta.3] - 2026-10-07

### Changed

- Filtering, sorting and folding a very large list no longer freeze the screen. The old list stays
  usable under a `rebuilding N%` readout while the new one is built in slices between frames.
- A tree toggle or sort change at a million keys reaches the new list in about a tenth of a second,
  and filters and folds are faster.
- Performance is now measured at a million keys in real `SCAN` order, not an idealised order.

## [0.1.0-beta.2] - 2026-10-07

### Added

- Redis Cluster support. The key list covers every primary, an open key stays live on whichever
  node owns it, and edits go to the owner.
- Cluster-aware Dashboard (with a node table), Slowlog and Monitor merged across nodes, and
  sharded Pub/Sub.

### Changed

- A Cluster target is no longer refused at startup.

## [0.1.0-beta.1] - 2026-10-04

### Added

- Themes: dark, light, high-contrast and your own.
- An ASCII fallback for terminals without Unicode.
- Session restore: your pane split, filter, sort and selected key return when you relaunch against
  the same server.
- Million-key browsing: the scan, filter, tree and metadata paths stay responsive at 1,000,000 keys.

### Fixed

- The value cursor is cleared whenever the value pane loses focus.

## [0.1.0-alpha] - 2026-09-03 to 2026-10-01

Eighteen alpha releases (`0.1.0-alpha.1` to `0.1.0-alpha.18`) built the first usable version:

- Connection resolution from flags, Profiles, environment variables and defaults, with the target
  and its source always shown in the title bar, secrets kept out of the config file, and TLS.
- A streaming, virtualized key list using `SCAN`, with tree and flat views, filtering and sorting.
- Viewers for every Redis type, with live updates for the open key pushed by the server.
- In-place editing of every core type, TTL changes and key deletion, each previewed as the exact
  command, with Environments and Read-only Mode.
- Monitor, Pub/Sub, Slowlog and Dashboard views.
- Clipboard copy, including OSC 52 over SSH, and prebuilt binaries for macOS, Linux and Windows.

[Unreleased]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-beta.6...HEAD
[0.1.0-beta.6]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-beta.5...v0.1.0-beta.6
[0.1.0-beta.5]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-beta.4...v0.1.0-beta.5
[0.1.0-beta.4]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-beta.3...v0.1.0-beta.4
[0.1.0-beta.3]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-beta.2...v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-beta.1...v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/redis-pane/redis-pane/compare/v0.1.0-alpha.18...v0.1.0-beta.1
[0.1.0-alpha]: https://github.com/redis-pane/redis-pane/releases/tag/v0.1.0-alpha.18
