---
title: "Environment variables"
---

## Redis target

| Variable | Used for |
| --- | --- |
| `REDIS_URL` | A whole target. Used as written, and never merged with the variables below. |
| `REDIS_HOST` | The host, when `REDIS_URL` isn't set. Default `127.0.0.1`. |
| `REDIS_PORT` | The port, when `REDIS_URL` isn't set. Default `6379`. |
| `REDIS_USER` | The ACL username. |
| `REDIS_PASSWORD` | The password. |

These come after flags and the default Profile in
[the order the target is chosen](../connecting/target.md). Empty variables count as not set.

`REDIS_USER` and `REDIS_PASSWORD` apply to a target from flags, from `REDIS_URL` or
`REDIS_HOST`, or from the default. They don't apply to a Profile, which has its own credentials.
`--user` and `--password` always win.

## Locations

| Variable | Used for |
| --- | --- |
| `XDG_CONFIG_HOME` | Where the [config file](./configuration.md) is: `$XDG_CONFIG_HOME/redis-pane/config.json`. Default `~/.config`. |
| `XDG_STATE_HOME` | Where [session restore](../browsing/session-restore.md) keeps `redis-pane/state.json`. Default `~/.local/state`. |

## Display

| Variable | Used for |
| --- | --- |
| `NO_COLOR` | Set to anything but empty, it selects monochrome. |
| `COLORTERM` | `truecolor` or `24bit` selects truecolor. |
| `TERM` | Selects 256 colours, or monochrome if it's `dumb` or unset. |
| `WT_SESSION` | Set by Windows Terminal, which gets truecolor. |
| `LC_ALL`, `LC_CTYPE`, `LANG` | The first one set decides ASCII or Unicode. See [ASCII and colour](../appearance/ascii-and-colour.md). |

## Clipboard

| Variable | Used for |
| --- | --- |
| `SSH_TTY`, `SSH_CONNECTION`, `SSH_CLIENT` | Any one of them means an SSH session, so copying uses [OSC 52](../viewing/copying.md). |

## Passwords

Whichever variable a Profile's `passwordEnv` names is read when you connect. There is no fixed
name.
