# Demo tapes

The recordings on the docs site and in the README are rendered from the `.tape` files here with
[VHS](https://github.com/charmbracelet/vhs). They are rendered locally and committed; CI does not
render them. The only copy of the output is `site/public/demos/`.

## Prerequisites

- `brew install vhs` (this also pulls in `ttyd` and `ffmpeg`; install `ffmpeg` yourself if missing)
- Docker, running
- Python 3 (no pip: `scripts/fixtures.py` and `scripts/churn.py` use only the standard library)
- a Rust toolchain; `render.sh` builds `target/release/redis-pane` first

## Usage

```bash
./tapes/render.sh            # every tape
./tapes/render.sh hero       # one, or several: ./tapes/render.sh hero monitor
```

For each tape `render.sh` writes, into `site/public/demos/`:

| File | Used for |
|---|---|
| `<name>.webm`, `<name>.mp4` | the looping video the site plays (`<Demo name="..." />`) |
| `<name>.png` | the poster, a frame from the middle of the session |
| `<name>.gif` | only tapes marked `# gif: yes`; the README embeds these |

Budget: a `.webm` or `.mp4` is at most 1 MB and a `.gif` at most 1.5 MB. Aim for 8 to 20 seconds.

## What `render.sh` does, and what it never does

- Starts **its own** Redis, the container `redis-pane-demo` (`redis:8.4-alpine`) on port **6390**,
  and removes only that container when it finishes, including when it fails.
- Never touches `redis-pane-dev` (port 6379) or `redis-pane-cluster`. Every tape connects with an
  explicit `--url redis://127.0.0.1:6390`, and the helper scripts get `--port 6390`.
- Seeds the data the same way every time: `scripts/fixtures.py --flush --seed 7`, plus a few keys
  per scenario in `seed()`.
- Points `XDG_CONFIG_HOME` and `XDG_STATE_HOME` at a temp directory that is reset for each tape, so
  none of your Profiles, and no saved filter or scroll from a previous recording, can appear.
  A scenario that needs a Profile writes it into that temp config with `chmod 600`.
- Puts `target/release/redis-pane` first on `PATH`.
- Re-encodes the VHS master with `ffmpeg` into the webm, mp4, poster and (where marked) gif.

## Writing a tape

- Start with `Source "_settings.tape"` (the shared theme, size, font and typing speed), then
  `Output ../site/public/demos/<name>.mp4`. `render.sh` replaces that master with the encoded files.
- Add `# poster: <seconds>` (a time in the recording) and, for the README ones, `# gif: yes`
  to the header comment.
- Launch the app under `Hide` and `Show` once it is up, so the recording opens on the app and not on
  a half-typed command. Start background writers (`churn.py`) under `Hide` too, and kill them at the
  end of the tape under `Hide`.
- Take keys from `site/content/docs/reference/keybindings.md`. If a step does not do what its
  comment says, the tape is wrong. Extract a few frames with `ffmpeg` and look at them.
