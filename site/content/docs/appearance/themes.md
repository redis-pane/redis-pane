---
title: "Themes"
---

There are three built-in themes:

| Theme | For |
| --- | --- |
| `dark` | The default. It paints no background, so it blends with your terminal. |
| `light` | Dark text on a light background. It paints its own background. |
| `high-contrast` | The strongest contrast. It paints its own background. |

`light` and `high-contrast` paint their own background, so they stay readable whatever your
terminal's colours are.

## Choosing one

For one session:

```bash
redis-pane --theme light
```

For good, in the [config file](../connecting/profiles.md):

```json
{
  "theme": "light"
}
```

`--theme` beats the config file, and the config file beats the default (`dark`). A name that is
neither a built-in nor one of your [own themes](./custom-themes.md) is an error at startup, not a
silent fallback.

## Contrast

Every colour that carries text in the built-in themes is tested against the background it is
drawn on. `light` and `dark` meet the WCAG AA contrast ratio (4.5:1) and `high-contrast` meets
AAA (7:1). `dark` has one caveat: because it paints no background, it is only as readable as your
terminal's background is dark.

Meaning is never carried by colour alone. Every colour signal is paired with a word, a glyph or a
weight, so a theme with no colour at all, such as [`NO_COLOR`](./ascii-and-colour.md), loses
emphasis but not information.
