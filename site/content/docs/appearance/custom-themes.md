---
title: "Custom themes"
---

Define your own under `themes` in the [config file](../connecting/profiles.md). A theme names the
colours it changes and inherits the rest from a built-in `base`, which is `dark` unless you say
otherwise.

```json
{
  "theme": "mine",
  "themes": {
    "mine": {
      "base": "light",
      "border-focus": "#0b5cad",
      "selected": { "fg": "#000000", "bg": "#ffd27f" }
    }
  }
}
```

Then `redis-pane` uses it by default, or use `--theme mine` for one run.

## Colour values

- **`"#rrggbb"`** sets a token's foreground. For `selected`, `surface-detached` and
  `background`, it sets the background instead.
- **`{ "fg": "#rrggbb", "bg": "#rrggbb" }`** sets either or both.
- **`"terminal"`** is accepted for `background` only. It stops a painting base (`light`,
  `high-contrast`) from painting.

Only six-digit hex is accepted. No colour names, no `#rgb`, no alpha. In a 256-colour terminal
each colour is mapped to the nearest one available.

A theme inherits its base's background: `base: light` paints one, and `base: dark` doesn't.

## The tokens

Widgets ask for a role, not a colour, and a theme maps each role.

| Token | Used for |
| --- | --- |
| `text` | Primary content. |
| `muted` | Secondary content: TTLs, sizes, counts. |
| `border` | Pane edges. |
| `border-focus` | The focused pane's edge. |
| `selected` | The cursor row: a full bar with a fixed foreground and background. |
| `env.local`, `env.staging`, `env.prod`, `env.unknown` | The title bar band and confirm dialogs for each [Environment](../safety/environments.md). |
| `type.string`, `type.hash`, `type.list`, `type.set`, `type.zset`, `type.stream`, `type.json` | The hue for each type, wherever it appears. |
| `ok`, `warn`, `danger` | Success, expiring or alarming, and destructive. |
| `surface-detached` | The wash on the value pane while it holds a key that isn't the Selected key. |
| `background` | Painted behind everything, overlays included. |

The underline on the open key's name is not a colour, and can't be themed. A token name that
isn't in this list is an error, so a typo such as `bordr-focus` is caught, not ignored.

## Rules

- A theme can't have the name of a built-in (`dark`, `light`, `high-contrast`).
- `base` must be a built-in.
- `theme` must name a built-in or one of your themes.
- Your own themes aren't checked for contrast. The built-ins are.

## If you roll back

An older `redis-pane` that predates themes refuses a config file with `theme` or `themes` in it,
since an unknown field is an error. Remove them before going back to an older version.
