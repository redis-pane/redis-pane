# Environments

Every target has an **Environment**: `local`, `staging`, `prod` or `unknown`. It tells
`redis-pane` how careful to be, and it tells you, in the title bar, how careful to be.

| Environment | Title bar | Starts in |
| --- | --- | --- |
| `local` | Neutral | Writes allowed |
| `staging` | Amber | Writes allowed |
| `prod` | Red | [Read-only Mode](read-only.md) |
| `unknown` | A distinct fourth colour | [Read-only Mode](read-only.md) |

Colour is never the only signal. The Environment's name is spelled out next to the target, and
the confirmation dialogs carry it too. `unknown` is deliberately not a shade of the others,
because it means "nobody told us", not "somewhere between staging and prod".

## How it is decided

**A Profile declares it.** Set `env` on the [Profile](../connecting/profiles.md). Nothing is
inferred for a Profile. If you leave `env` out, you get `unknown`, never `local`.

**Anything else is inferred from the address.** A target you gave with flags, in the environment
or by default is:

- `local` if the host is loopback (`127.0.0.1`, `localhost`, `::1`) or a Unix socket, including a
  loopback Sentinel or Cluster URL.
- `unknown` for everything else.

This is why `redis-pane --url redis://cache-prod-01:6379` arrives read-only. That is exactly the
command people type in the middle of an incident.

There is no flag to set the Environment of an ad-hoc target. If you connect to something often
and want it to be `staging`, make it a Profile.

## Where you'll see it

- The title bar, in the band colour and in words.
- Confirmation dialogs.
- The typed-count step of a [bulk delete](../editing/bulk-delete.md) on `prod` and `unknown`.
- The ask before opening [Monitor](../server/monitor.md) on `prod` and `unknown`.
