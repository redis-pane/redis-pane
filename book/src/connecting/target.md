# How the target is chosen

`redis-pane` never prompts for a target. It works one out, using the first of these that
applies:

1. **Flags.** `--profile NAME` (or a bare name, `redis-pane staging`) picks a
   [Profile](profiles.md) and wins over everything below it. Otherwise `--url`, `--host` or
   `--port`, on their own or together, name a target.
2. **The default Profile**, `defaultProfile` in the config file.
3. **The environment.** `REDIS_URL`, or the discrete `REDIS_HOST` and `REDIS_PORT`.
4. **`127.0.0.1:6379`.**

With nothing configured, you get a local Redis. There is no setup step.

## The title bar tells you what it picked

The title bar always shows three things together: the target, its [Environment](../safety/environments.md),
and where the target came from.

| Source shown | Means |
| --- | --- |
| `from profile staging` | A Profile named `staging`, by flag or as the default. |
| `from flag` | `--url`, `--host` or `--port`. |
| `from environment` | `REDIS_URL`, `REDIS_HOST` or `REDIS_PORT`. |
| `from default` | Nothing else applied, so `127.0.0.1:6379`. |

Resolving silently is what makes zero configuration possible, and that readout is the safeguard
for it. Read it before you press a key that writes.

**A Profile you name has to exist.** If `--profile` (or a bare name) isn't defined in the config
file, `redis-pane` stops with an error that names it and lists the Profiles it does have, and exits
with code `2`. It does not fall through to the next rule, because a typo would then land you on a
local Redis. If there is no config file at all, the error says so.

```text
$ redis-pane --profile stagin
redis-pane: no Profile named "stagin"; available Profiles: prod, staging
```

## `REDIS_URL` is used whole

When `REDIS_URL` is set, it is the whole target. It is never mixed with `REDIS_HOST`,
`REDIS_PORT`, `REDIS_USER` or `REDIS_PASSWORD` left over in your shell. A stale `REDIS_PORT` can't
quietly redirect a `REDIS_URL`.

The same goes for `--url` and a Profile's `url`: they are used as written, never merged with
`host` or `port`.

## Credentials

Credentials are resolved separately from the target:

- `--user`, `--password` and `--tls` always win over a Profile and over the environment.
  `--tls` can only turn TLS on, never off.
- A Profile uses its own `username`, password reference and `tls`.
- For a target that came from flags, `REDIS_URL`, `REDIS_HOST` or the default, `REDIS_USER` and
  `REDIS_PASSWORD` supply the username and password if you haven't given them another way.

See [Passwords and secrets](passwords.md) before you put a password on the command line.

## Check without connecting

```bash
redis-pane --print-target
redis-pane --profile staging --print-target
```

`--print-target` prints the same line as the title bar, for example
`cache-01:6379/0 · staging · from profile staging`, and exits without connecting. Passwords in a
URL are masked.

`--probe` connects, reports what the server supports, and exits:

```bash
redis-pane --profile staging --probe
```

It prints the target, the Redis version, whether live updates are available
(`CLIENT TRACKING accepted` or `refused`), and, on a Cluster or a replica, what it found there.
Use it to check a new Profile without opening the full UI.

## When it can't connect

A target that can't be reached exits to your shell with a message naming the target, its Source
and the reason. It does not open an empty error screen. The exit code tells scripts why:

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `2` | The target couldn't be reached, or refused the credentials, or the Profile you named on the command line isn't defined. |
| `3` | The [config file](profiles.md) was malformed or refused. |
| `4` | The server is older than Redis 6.0, or doesn't speak RESP3. |
