# Command-line options

> Generated from the `clap` definition; do not edit. Regenerate with `UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference`.

`redis-pane` takes a target from flags, or finds one. With no arguments it uses, in order:

1. flags (`--profile`, `--url`, `--host`, and the rest, below)
2. the default Profile in the config file
3. the environment (`REDIS_URL`, or the discrete `REDIS_HOST`/`REDIS_PORT`/`REDIS_USER`/`REDIS_PASSWORD`)
4. `127.0.0.1:6379`

It never prompts. The title bar always shows the target and where it came from.

```text
A terminal UI for Redis

Usage: redis-pane [OPTIONS] [PROFILE]

Arguments:
  [PROFILE]
          Profile name, positionally

Options:
      --profile <NAME>
          Profile to use, by name. A bare positional name works too

      --url <URL>
          Connect to this URL, used wholesale

      --host <HOST>
          

      --port <PORT>
          

      --db <N>
          Database index. Fixed at launch; there is no in-app switcher (ADR-0005)

      --user <NAME>
          ACL username. Always wins over a Profile's or the environment's

      --password <SECRET>
          Password, given directly. Visible in shell history and to other users via `ps` — prefer a Profile's `passwordEnv`/`passwordCommand` for anything long-lived. Always wins over a Profile's or the environment's

      --tls
          Use TLS. Additive only — there is no --no-tls to downgrade a Profile or a `rediss://` URL that already wants it

      --ascii
          Draw with ASCII glyphs only. The default follows the locale: ASCII unless it declares UTF-8. Overrides the config file's `ascii`

      --unicode
          Draw with Unicode glyphs even where the locale does not declare UTF-8

      --print-target
          Resolve and print the target, then exit without connecting

      --probe
          Connect, report what the server supports, then exit

      --theme <NAME>
          Colour theme: `dark`, `light`, `high-contrast`, or one defined under `themes` in the config. Beats the config's `theme`

  -h, --help
          Print help

  -V, --version
          Print version
```
