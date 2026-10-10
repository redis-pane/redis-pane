---
title: Command-line options
---

> Generated from the `clap` definition; do not edit. Regenerate with `UPDATE_DOCS=1 cargo test -p redis-pane --test docs_reference`.

`redis-pane` takes a target from flags, or finds one. With no arguments it uses, in order:

1. flags (`--profile`, `--url`, `--host`, and the rest, below)
2. the default Profile in the config file
3. the environment (`REDIS_URL`, or the discrete `REDIS_HOST`/`REDIS_PORT`/`REDIS_USER`/`REDIS_PASSWORD`)
4. `127.0.0.1:6379`

It never prompts. The title bar always shows the target and where it came from.

```text
A terminal UI for Redis.

The target is chosen in this order: flags (--profile, --url, --host and --port), then the default Profile in the config file, then the environment (REDIS_URL, or REDIS_HOST, REDIS_PORT, REDIS_USER and REDIS_PASSWORD), then 127.0.0.1:6379. It never prompts; the title bar always shows the target and where it came from.

A Profile named on the command line that the config file doesn't define is an error, not a fallback.

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
          Connect to this host. Combine with --port; the port defaults to 6379

      --port <PORT>
          Connect to this port. Combine with --host; the host defaults to 127.0.0.1

      --db <N>
          Database index. The database is fixed at launch; there is no way to change it from inside the app

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
          Print help (see a summary with '-h')

  -V, --version
          Print version
```
