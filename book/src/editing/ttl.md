# TTL

Press `t` in the value pane to change the open key's TTL. You type a duration, not a number of
seconds, and the field tells you, as you type, which of four things it will do.

| You type | It does |
| --- | --- |
| `5m`, `2h30m`, `1d` | **Set** the TTL to that long from now. |
| `+30m` | **Extend** the TTL by 30 minutes. |
| `-10m` | **Shorten** the TTL by 10 minutes. |
| `never`, or nothing | **Persist**: remove the expiry. |

A bare number is seconds. Units are `s`, `m`, `h` and `d`, largest first, so `1d2h30m10s` works
and `30m2h` doesn't. Case and surrounding spaces don't matter.

`⌃S` applies the TTL and shows the command. `y` runs it. `Esc` cancels.

## What it refuses, and why

The field says why under the text, and `⌃S` stays blocked until you fix it.

- **`0`** is refused. In Redis, a TTL of 0 deletes the key. A TTL field that deletes a key when
  you type one character would be a delete in disguise. Use `d`, which previews `DEL` and asks for
  confirmation.
- **`+30m` or `-10m` on a key with no expiry** is refused, since there's nothing to extend. Type
  `30m` to set one.
- **A shorten that would pass zero** is refused, not rounded up.
- **More than about 68 years** is refused.
- **Text it can't read**, like `1.5h` or `5 m`.

Extend and shorten are applied to the TTL **as the server sees it when the change lands**, not the
TTL you saw when you opened the dialog. A slow confirm doesn't skew them.

## How it counts down

The TTL in the header counts down on its own, between reads, with no network cost. Every read
corrects it from the server. It can read a little ahead or behind the server for a moment, never
for long.
