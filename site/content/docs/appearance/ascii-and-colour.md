---
title: "ASCII and colour"
---

## ASCII

If a glyph shows as a box, a question mark or a gap, run:

```bash
redis-pane --ascii
```

That swaps every symbol for a plain-ASCII one of the same width, so the layout doesn't move:
`●` becomes `*`, `▾` becomes `v`, box lines become `+ - |`, and so on. `redis-pane` uses no Nerd
Font glyphs and no emoji, so you don't need a special font.

The choice is made in this order:

1. `--ascii` or `--unicode` on the command line.
2. `"ascii": true` or `"ascii": false` in the [config file](../reference/configuration.md).
3. Your locale. Unicode is used only if the first of `LC_ALL`, `LC_CTYPE` and `LANG` that is set
   names UTF-8. If none is set, or the locale is `C` or `POSIX`, you get ASCII. Windows has no
   such variables and gets Unicode.

The locale rule is aimed at SSH, where the environment you forward is often empty. Plain text
is better than question marks. If you are getting ASCII and want Unicode, pass `--unicode`.

## Colour

`redis-pane` uses the most colour your terminal says it supports: truecolor, then 256 colours,
then none.

- **`COLORTERM=truecolor`** (or `24bit`) selects truecolor. So does Windows Terminal.
- **Otherwise you get 256 colours**, unless `TERM` is `dumb`, or `TERM` is unset on macOS or
  Linux. Those give monochrome. On Windows, an unset `TERM` still gets 256 colours.
- **`NO_COLOR`**: if it is set to anything other than an empty string, you get monochrome,
  whatever the terminal claims. This follows [no-color.org](https://no-color.org).

In monochrome, nothing is lost: the focused pane is shown by a bold title, the cursor by reverse
video, and the type name stays in the key list. Themes don't paint a background in monochrome.
