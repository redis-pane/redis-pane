---
title: "Profiles and the config file"
---

A **Profile** is a named target you've written down: an address, a way to find the password,
an Environment and an optional note. Use one for anything you connect to more than once.

## Where the file lives

`redis-pane` reads `~/.config/redis-pane/config.json`. If `XDG_CONFIG_HOME` is set, it reads
`$XDG_CONFIG_HOME/redis-pane/config.json` instead.

The file is optional. Without it, `redis-pane` runs on flags, the environment and the default.

**`redis-pane` only reads this file.** It has no "save profile" command. You write the file by
hand, and you can keep it in a dotfiles repository. Anything the app remembers for itself, such
as your filter and sort, goes in a separate [state file](../browsing/session-restore.md).

## A first Profile

```json
{
  "defaultProfile": "staging",
  "profiles": {
    "staging": {
      "host": "cache-01.staging.example.com",
      "port": 6379,
      "passwordEnv": "STAGING_REDIS_PASSWORD",
      "env": "staging",
      "note": "shared staging cache"
    }
  }
}
```

Then:

```bash
export STAGING_REDIS_PASSWORD=...
redis-pane            # uses the default Profile
redis-pane staging    # names it
```

Every field is described in the [configuration reference](../reference/configuration.md).

## Always set `env`

A Profile with no `env` gets the Environment `unknown`, never `local`. `unknown` is a real
Environment, and it starts in [Read-only Mode](../safety/read-only.md). Set `env` to `local`,
`staging` or `prod` so the title bar and the safety rules match what you are connecting to. See
[Environments](../safety/environments.md).

## The file is strict

Two rules protect you from a silent mistake.

**An unknown field is an error.** A misspelled `passwordEnv` would otherwise mean the password is
quietly dropped and you'd get an authentication failure that doesn't point at the typo. So
`redis-pane` refuses to start and names the line and column:

```text
redis-pane: ~/.config/redis-pane/config.json: line 5, column 24: unknown field `passwrodEnv`, expected one of `url`, `host`, ...
```

**The file must be private.** If it is readable by your group or by anyone else, `redis-pane`
refuses to read it and tells you to run `chmod 600` on it. The check applies to every config
file, not only to ones that hold a password, because the file is the kind of thing a password
ends up in. It follows the same rule `ssh` applies to private keys. It is checked on macOS and
Linux only.

A broken config never falls back to localhost. You get the error and exit code `3`.
