# Configuration

`redis-pane` reads `~/.config/redis-pane/config.json`. It never writes it. Unknown fields are
refused, so a misspelled `passwordEnv` is an error and not a password silently dropped.

The file is optional. See [Profiles and the config file](../connecting/profiles.md) for how to
write one, and [How the target is chosen](../connecting/target.md) for when a Profile is used.

## Where it's read from

- `$XDG_CONFIG_HOME/redis-pane/config.json` if `XDG_CONFIG_HOME` is set and not empty.
- Otherwise `~/.config/redis-pane/config.json`.

On macOS and Linux the file must not be readable by your group or by others. `chmod 600` it, or
`redis-pane` refuses to read it and exits with code `3`. The file is JSON: no comments and no
trailing commas. Use a Profile's `note` where you'd have used a comment.

## A complete example

A complete example with two Profiles:

```json
{
  "defaultProfile": "local",
  "profiles": {
    "local": {
      "host": "127.0.0.1",
      "port": 6379,
      "env": "local"
    },
    "staging": {
      "url": "rediss://cache-01.staging.example.com:6380/0",
      "username": "ops",
      "passwordEnv": "STAGING_REDIS_PASSWORD",
      "env": "staging",
      "note": "shared staging cache"
    }
  },
  "ascii": false,
  "theme": "dark"
}
```

## Top-level fields

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `defaultProfile` | string | none | The Profile used when you name no target. Must be a key of `profiles`. |
| `profiles` | object | `{}` | Your Profiles, by name. |
| `ascii` | boolean | follows the locale | `true` draws ASCII glyphs only, `false` draws Unicode. `--ascii` and `--unicode` override it. See [ASCII and colour](../appearance/ascii-and-colour.md). |
| `theme` | string | `dark` | The theme to use: a built-in name or a key of `themes`. `--theme` overrides it. |
| `themes` | object | `{}` | Your own themes, by name. See [Custom themes](../appearance/custom-themes.md). |

## Profile fields

Every field is optional. A Profile with neither `url` nor `host` connects to `127.0.0.1`.

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `url` | string | none | A full URL (`redis://`, `rediss://`, `redis-sentinel://`, `redis-cluster://` and the `rediss-` forms). Used as written and never merged with `host` or `port`. |
| `host` | string | `127.0.0.1` | The server's address. |
| `port` | integer, 0 to 65535 | `6379` | The server's port. |
| `db` | integer, 0 to 255 | `0` | The database. Applies with `host` and `port`; with a `url`, put the database in the URL. See [Databases](../connecting/databases.md). |
| `username` | string | none | The ACL username. |
| `password` | string | none | A literal password. The file must be private. Prefer a reference. |
| `passwordEnv` | string | none | The name of an environment variable that holds the password. |
| `passwordCommand` | string | none | A command whose first line of output is the password. |
| `tls` | boolean | `false` | Use TLS. A `rediss://` URL does too. |
| `note` | string | none | Free text, shown next to the Connection. JSON has no comments, so this stands in for one. |
| `env` | `"local"`, `"staging"`, `"prod"` or `"unknown"` | `"unknown"` | The [Environment](../safety/environments.md). Lower case only. |

If more than one password field is set, `passwordEnv` wins, then `passwordCommand`, then
`password`. See [Passwords and secrets](../connecting/passwords.md).

## Theme fields

A theme is an object. Its keys are `base` and any of the colour tokens.

| Key | Value | Meaning |
| --- | --- | --- |
| `base` | `"dark"`, `"light"` or `"high-contrast"` | The built-in theme to inherit from. Default `dark`. |
| a token name | `"#rrggbb"` or `{ "fg": "#rrggbb", "bg": "#rrggbb" }` | The colour for that token. `background` also accepts `"terminal"`. |

The token names are listed in [Custom themes](../appearance/custom-themes.md#the-tokens).

## What is refused

Each of these stops `redis-pane` at startup with a message that names the file, the line and the
column where it can. It never falls back to localhost.

**A misspelled field:**

```json,invalid
{
  "profiles": {
    "staging": {
      "host": "cache-01.staging.example.com",
      "passwrodEnv": "STAGING_REDIS_PASSWORD"
    }
  }
}
```

Unknown fields are errors because you wrote this file by hand, and a typo in `passwordEnv` would
otherwise mean a missing password and an authentication failure that points nowhere near the
typo.

**A default Profile that doesn't exist:**

```json,invalid
{
  "defaultProfile": "prod",
  "profiles": {
    "staging": { "host": "cache-01.staging.example.com", "env": "staging" }
  }
}
```

**An Environment in the wrong case, or one that doesn't exist:**

```json,invalid
{
  "profiles": {
    "staging": { "host": "cache-01.staging.example.com", "env": "Staging" }
  }
}
```

**A theme that doesn't exist, or one that shadows a built-in:**

```json,invalid
{
  "theme": "solarized"
}
```

Also refused: a theme named `dark`, `light` or `high-contrast`, a `base` that isn't a built-in, a
token name that isn't in the list, and a colour that isn't `#rrggbb`.
