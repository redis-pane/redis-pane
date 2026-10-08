# Passwords and secrets

A Profile can hold a password three ways. Use the first two for anything long-lived.

## `passwordEnv`

Names an environment variable that holds the password:

```json
{
  "profiles": {
    "staging": {
      "host": "cache-01.staging.example.com",
      "username": "ops",
      "passwordEnv": "STAGING_REDIS_PASSWORD",
      "env": "staging"
    }
  }
}
```

If the variable isn't set, `redis-pane` says so and exits. It tells you the variable's name,
never a value.

## `passwordCommand`

Names a command whose output is the password, so it can come from your password manager:

```json
{
  "profiles": {
    "prod": {
      "host": "redis.prod.example.com",
      "username": "readonly",
      "passwordCommand": "pass show redis/prod",
      "env": "prod"
    }
  }
}
```

The command runs through `sh -c` when you connect. Only the first line of its output is used,
with the trailing newline removed. If it fails, the message gives only the exit status. The
command and its error output are left out on purpose, because either can contain the secret.

## A literal `password`

```json
{
  "profiles": {
    "scratch": { "host": "10.0.0.9", "password": "hunter2", "env": "staging" }
  }
}
```

It works, but the secret sits in a file. That is why [the config file must be private](profiles.md#the-file-is-strict).
Prefer a reference.

If a Profile has more than one, the order is `passwordEnv`, then `passwordCommand`, then
`password`.

## `--password` on the command line

```bash
redis-pane --host your-host --user default --password your-password --tls
```

This works, and prints a warning to stderr every time. The password is visible in your shell
history and, through `ps`, to other users on the same machine. Use it for a quick look, not as
a habit.

## Where `REDIS_PASSWORD` fits

`REDIS_PASSWORD` is read for targets that come from flags, `REDIS_URL`, `REDIS_HOST` or the
default. A Profile doesn't use it. See [Environment variables](../reference/environment.md).

## What is never shown

Passwords are never written to a log, the title bar, the state file or an error message. A
password inside a URL (`redis://user:secret@host`) is masked as `user:•••@host` wherever the
target is displayed.
