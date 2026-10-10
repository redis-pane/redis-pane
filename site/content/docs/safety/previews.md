---
title: "Previews and confirmation"
---

Every change goes through the same steps, whatever it changes:

1. **Stage.** You press the key for the change (`e`, `d`, `R` and so on) and finish any editing.
2. **Preview.** A dialog shows the literal Redis command that will be sent.
3. **Confirm.** `y` runs it. `Esc` throws it away.

Only `y` confirms and only `Esc` dismisses. Any other key is ignored, so a stray keystroke can
never confirm a change or discard one.

## What the preview shows

The real command, for example `DEL user:8812:session` or `HSET user:1 token`, plus a muted line
naming what it checks first, such as *only if the field still exists · keeps its TTL*. Some
changes are sent as a small guarded script so that they can't recreate a key that has been
deleted, or overwrite something that appeared since you looked. The preview shows the command it
performs, not the script.

A change to a value shows `old → new`.

## Friction scales with the blast radius

- Deleting one key on `local` or `staging` takes one `y`.
- A [bulk delete](../editing/bulk-delete.md) on `prod` or `unknown` makes you type the number of
  keys first.
- On `prod` and `unknown`, you're in [Read-only Mode](./read-only.md) to begin with.

## Errors name the command

If a change fails, a notification names the Redis command that failed and what the server said.
Nothing fails silently.
