# M4 task 6: ASCII glyph fallback

Status: **done.** Outcome at the end of this document.

## Context

PLAN.md M4 row 6: "ASCII glyph fallback · Proves: every hard-coded glyph (`✕ ⊘ ✎ ▌ ● ○ ⁎ ⟡ ▶`,
sparkline and bar blocks, box drawing where needed) goes through one glyph set with a Unicode and
an ASCII variant. A test enforces width-identical variants. ASCII is chosen by config or flag, or
automatically when the locale isn't UTF-8. Goldens exist in ASCII mode."

DESIGN §5 states the intended rule plainly: "Nerd Font glyphs when detected, ASCII fallbacks
otherwise, chosen so the layout width does not change between the two." No such toggle exists
today:

- **Glyphs are hard-coded character literals**, confirmed present in `crates/core/src/render/mod.rs`
  and `crates/core/src/render/keys.rs` (six distinct literal-glyph patterns matched across the two
  files, covering the `✕ ⊘ ✎ ▌ ● ○ ⁎ ⟡ ▶` family PLAN's row names).
- **`crates/core/src/render/dashboard.rs`'s own doc comments say so directly**: `bar`'s comment
  reads "this crate has no Nerd-Font/ASCII toggle today (`theme::Token` only ever degrades colour,
  never a glyph), so both draw unconditionally," and `sparkline`'s says its block-element ladder
  "render[s] identically wherever UTF-8 does" — true, but offers no fallback for a non-UTF-8
  locale. `bar` uses `█`/`░`; `sparkline` uses the eight-level ladder `▁▂▃▄▅▆▇█`. Both are plain
  Unicode block elements, not Nerd Font glyphs, so they are a *UTF-8* concern (ASCII locale) rather
  than a *Nerd Font detection* concern — the two fallback triggers PLAN's row names ("ASCII is
  chosen by config or flag, or automatically when the locale isn't UTF-8") are doing double duty
  for two different reasons a fallback might be needed, and this task's glyph table should carry
  both: Nerd-Font-vs-ASCII icons, and Unicode-block-vs-ASCII bars/sparklines/box-drawing.
- **`theme::Token` only ever degrades colour** (`ColorDepth`), confirmed by reading `theme/mod.rs`
  — there is no analogous capability axis for glyph choice. This task adds that second axis
  alongside, not inside, `Theme`.
- **Box drawing** — ratatui's own `Block`/`Borders` widgets draw pane borders using Unicode box-
  drawing characters by default; "some terminals lack it" (the task brief's framing) means this
  task's glyph set should include an ASCII box-drawing variant (`+`, `-`, `|`) for borders too, not
  just the named icon set — **confirm at build time** whether this is a ratatui-level border-set
  swap (ratatui supports custom `BorderType`/`Set` symbols) or requires the app to pass its own
  `symbols::border::Set` through wherever `Block::default().borders(...)` is constructed.

## Decisions

1. **A `Glyphs` table, passed like the theme.** `crates/core/src/theme/` (or a sibling module,
   e.g. `crates/core/src/glyphs/mod.rs` — **confirm at build time** which; keeping it alongside
   `theme` makes sense since both are "injected presentation data" per the same architectural
   pattern, but it is a genuinely separate concern — colour depth and Unicode support are
   independent capabilities a terminal can have in any combination) holding one Unicode and one
   ASCII variant per named glyph role (`deleted`, `editing`, `live`, `manual`, `warn`, `pointer`,
   etc. — named by *role*, the same "semantic token, never a literal" discipline CLAUDE.md already
   states for colour, extended to glyphs). `render` functions ask the `Glyphs` table for a role and
   get back whichever variant is active, the same shape `Theme::style(Token)` already has for
   colour.
2. **A width-identity test.** Every glyph role's Unicode and ASCII variants must occupy the same
   terminal column width — most of the named glyphs (`✕ ⊘ ✎ ▌ ● ○ ⁎ ⟡ ▶`) are single-width, so
   their ASCII equivalents (`x`, `0`/`⊘`→`x` or similar, `*`, `|`, `*`/`o`, `o`, `*`, `*`, `>` —
   **confirm at build time** the exact ASCII character chosen per role; DESIGN §5 already settled
   one instance of this exact decision for the Ad-hoc Connection marker, `~` not `⚡`, which is a
   useful precedent: pick the plainest ASCII character that still reads as the right shape, not a
   multi-character approximation) are trivially width-1 already, but the test should use an actual
   width calculation (unicode-width crate, or ratatui's own cell-width helpers if it exposes one —
   **confirm at build time** which is already a dependency or more idiomatic here) rather than
   assuming `.chars().count() == 1` is sufficient, since a handful of candidate glyphs in this
   family could in principle be ambiguous-width in some terminfo databases. The sparkline and bar
   block-element ladders need their own width check too: each level must be single-width in both
   variants (ASCII fallback for the 8-level ladder is plausibly a smaller fixed set, e.g.
   `.:-=+*#%@` or similar ASCII "shading" convention — **confirm at build time**).
3. **ASCII box-drawing.** A second `Glyphs`-table-adjacent (or integrated) border character set:
   `+`/`-`/`|` for corners/edges, matching ratatui's `symbols::border::Set` shape so it can be
   passed directly to whatever `Block` construction already exists, rather than redrawing borders
   by hand.
4. **Selection: `--ascii` flag, config field, or automatic non-UTF-8-locale detection** — same
   precedence shape as the theme task (flag > config > automatic detection > Unicode default),
   consistent with this task reusing `m4-themes.md`'s "presentation as data" pattern (PLAN's own
   risk-order note: "both reshape how render asks for presentation and 6 reuses 5's pattern").
   Locale detection reads `LANG`/`LC_ALL`/`LC_CTYPE` (in that precedence, matching POSIX locale
   resolution order) for a `UTF-8`/`utf8` suffix; its absence or an explicit `C`/`POSIX` locale
   triggers ASCII automatically. **Confirm at build time** the exact detection function's shape —
   likely shell-side (`crates/app/src/terminal.rs`, alongside `detect_color_depth`/
   `resolve_color_depth`, following that exact precedent: a `detect_...`/`resolve_...` pure-logic
   split so the resolution function itself is unit-testable without real env vars).
5. **Goldens exist in ASCII mode** — at least one full-screen frame (again, the keys pane, matching
   the light/high-contrast theme goldens' precedent) rendered with the ASCII `Glyphs` table active,
   proving layout does not shift between the two variants.

## Architecture

Entirely core-side for the `Glyphs` table itself, the width-identity test, and every render
function's glyph lookup — pure data and pure rendering, same as `Theme`. Shell-side only for
detection (`--ascii` flag parsing, locale env-var reads in `terminal.rs`, mirroring
`detect_color_depth`/`resolve_color_depth`'s existing split between "gather the environment" and
"decide from it"), and for threading the resolved `Glyphs` table into `render::frame` alongside
`Theme`, the same way `Theme` and the `Clock` are already both passed in rather than looked up
globally (CLAUDE.md: "The clock is injected... Terminal capability degrades gracefully").

## Files touched

| File | Change |
|---|---|
| `crates/core/src/theme/` or new `crates/core/src/glyphs/mod.rs` | `Glyphs` table: named roles, Unicode + ASCII variant each |
| `crates/core/src/render/mod.rs` | hard-coded glyph literals replaced with `Glyphs` lookups |
| `crates/core/src/render/keys.rs` | same |
| `crates/core/src/render/dashboard.rs` | `bar`/`sparkline` gain ASCII-ladder variants; doc comments updated (both currently state outright that no toggle exists) |
| `crates/core/src/render/layout.rs` or wherever `Block`/border construction lives | ASCII box-drawing `symbols::border::Set` wired through |
| `crates/app/src/terminal.rs` | `--ascii` flag / locale detection, alongside `detect_color_depth` |
| `crates/core/src/config/mod.rs` | `ascii: Option<bool>` (or similarly named) config field, additive like `theme` |
| `crates/core/tests/golden.rs` | ASCII-mode golden frame(s) |
| `docs/DESIGN.md` §5 | record the glyph-fallback mechanism now that it exists, since §5 currently only promises it in prose |

## Testing

- **Core unit tests**: every `Glyphs` role's Unicode and ASCII variants are equal-width (the
  width-identity test PLAN's row names explicitly); glyph lookups for both variants return the
  correct character for every role used in `render/`.
- **Shell unit test**: locale-based ASCII auto-detection resolves correctly across
  `LANG=en_US.UTF-8` (Unicode), `LANG=C`/`LANG=POSIX` (ASCII), and an unset `LANG` with no
  `LC_ALL`/`LC_CTYPE` fallback (ASCII, conservatively — **confirm at build time** whether "unset
  entirely" should default to Unicode or ASCII; given this app's SSH-bastion-first use case,
  defaulting conservatively to ASCII when nothing claims UTF-8 support is probably the safer
  choice, matching `resolve_color_depth`'s already-conservative stance on absent `TERM`).
- **Golden frames**: at least one full frame in ASCII mode, proving layout parity with the Unicode
  equivalent of the same state — this is the test that actually proves "layout width does not
  change between the two," not just the per-glyph width-identity unit test in isolation.
- **A by-hand script**, per the task brief's own suggestion: `LANG=C cargo run -p redis-pane`
  against fixture data, to catch anything that reads correctly in a unit test but looks wrong in a
  real terminal (e.g. a font that happens to render an ASCII substitute ambiguously).

## CLAUDE.md rules this binds

- **Same for icons [as colors]: widgets ask for a role, never a hex value — or, here, never a
  literal glyph.** This task is the direct build-out of that already-stated rule; today's code
  violates it by literal, and `render/dashboard.rs`'s own comments say so.
- **Terminal capability degrades gracefully... Nerd Font → ASCII.** Named explicitly in CLAUDE.md's
  architecture-guidance list; this task is that line item.
- **Colors are semantic tokens, never literals. Same for icons (Nerd Font vs. ASCII must be
  width-identical).** The width-identity requirement is already stated as a hard rule in CLAUDE.md,
  not just DESIGN prose — this task's test is what makes that rule enforced rather than aspirational.

## Out of scope

- **Screen-reader-friendly linearized rendering** (DESIGN §8's separate accessibility item) —
  box-drawing removal there is about a fundamentally different render path (linear text, not a
  bordered grid), not the ASCII-vs-Unicode glyph substitution this task builds.
- **A user-facing glyph customization surface** beyond the Unicode/ASCII toggle — no "pick your own
  icon set," matching the themes task's equivalent out-of-scope call (no in-app picker UI); this is
  a capability-degradation mechanism, not a personalization feature.

## Outcome

Built as planned, with the "confirm at build time" choices below and the deviations after them.

**Confirm-at-build-time choices**

| Question | Choice |
|---|---|
| Where the table lives | `crates/core/src/glyphs.rs`, a sibling of `theme/`, not inside it |
| Width calculation | `ratatui::text::Line::width` (already a dependency; it is `unicode-width` underneath), plus a `chars().count() == 1` check |
| Box drawing | The renderer draws every border by hand (no `Block`/`Borders` anywhere), so there was no `symbols::border::Set` to swap; each corner, rule and tee is a `Glyph` role instead |
| ASCII per role | `✕✗`→`x`, `✓✎`→`+`, `●⁎•`→`*`, `○⟡`→`o`, `⊘`→`/`, `⟳`→`@`, `⚠`→`!`, `▏▌│`→`\|`, `▶▸→`→`>`, `▾▼↓`→`v`, `▲↑⌃`→`^`, `←⏎⌫`→`<`, `⌥`→`M`, `…∞`→`~`, `·—─░`→`-` (pending cell `·`→`.`), `µ`→`u`, `┌┐└┘├`→`+`, `┊`→`:`, `┈`→`.`, `█`→`#` |
| Sparkline ladder | `_.:-=+*#`, eight levels, ascending in visual weight, none blank |
| Locale, unset entirely | ASCII (the plan's recommendation). Windows has no POSIX locale variables and is treated as Unicode, as `resolve_color_depth` treats an absent `TERM` there |
| Config field | top-level `ascii: Option<bool>`; the flag is `--ascii`, with `--unicode` to force the other way against a config or locale that says ASCII |
| Precedence | flag > config > locale |

**Deviations from the plan**

- **The glyph set rides on `Theme`** (`theme.glyphs`, `Theme::with_glyphs`), not beside it as a
  separate argument to `render::frame`. Every draw function already receives a `&Theme`, so this
  changed no signature and no call site; the plan's "alongside, not inside" intent (colour depth
  and glyph set stay independent capabilities) is kept by `Glyphs` being its own type that
  `Theme` merely carries.
- **`Glyphs::text(&str)`** exists beside role lookups. Several glyphs are baked into strings the
  core's *state* builds (`Liveness::readout`, `ServerCondition::readout`, the Viewer's currency
  line, key labels, help rows, hint bar) and which tests assert byte for byte, so they stay
  Unicode in state and are mapped at the draw site. Because every variant is one column wide,
  truncation decisions made on the Unicode text are still right. It is never applied to a key
  name, value, command, channel or payload.
- **The enforcement is a frame-level test**, not role discipline alone:
  `the_ascii_frame_has_the_unicode_frames_layout_and_no_non_ascii` renders eleven states in both
  sets and asserts equal row widths and ASCII-only output, so a draw site that still holds a
  literal glyph fails the build.
- `truncate`, `clip`, `truncate_left` and `truncate_right` take the ellipsis (or a `Glyphs`) as
  an argument, so a truncated key name gets `~` without the name itself being mapped.

**Not done**

- A stream entry whose ID does not parse shows `—` in the AGE cell (`state::value::stream_entry_age`);
  the Viewer's row cells carry no glyph context, so that one cell stays Unicode in an ASCII frame.
- The by-hand `LANG=C cargo run -p redis-pane` check against the fixtures was not run in this
  environment (no interactive terminal); the golden frames in ASCII mode stand in for it.
- The integration suite was not run (it needs Docker).
