# M4 task 5: Themes

Status: **done.** Decisions confirmed by the user 2026-10-04: `EnvLocal` becomes a neutral
grey (code brought in line with DESIGN §5); `Token` names are kept and DESIGN §5's role table is
updated to name the tokens that exist (no rename).

## Context

PLAN.md M4 row 5: "Themes · Proves: `Theme` becomes data: a token→colour palette per colour
depth. Built-ins are dark (today's), light and high-contrast, all checked for WCAG AA contrast by
a test. The theme is chosen by an additive config field `theme` and a `--theme` flag. User themes
are token→colour maps in config, unknown tokens a parse error. Goldens exist for light and
high-contrast (ADR-0011's promise). `NO_COLOR` treats an empty value as unset, per no-color.org."

DESIGN §5 promises "a dark default and a light default, both truecolor, both contrast-checked...
Themes are data (a token → color map in config), so users can add their own." None of that is true
today:

- **`crates/core/src/theme/mod.rs`'s `Theme` is `{ depth: ColorDepth }`** — one struct, three
  hard-coded `match` functions (`true_color`, `ansi_256`, `monochrome`), each a literal `Color::Rgb`/
  `Color::Indexed` per `Token`. There is no palette data structure, no second theme, nothing a user
  could override.
- **`Token` has 21 variants** (counted directly from the enum: `Text`, `Muted`, `Border`,
  `BorderFocus`, `Selected`, `EnvLocal`, `EnvStaging`, `EnvProd`, `EnvUnknown`, `TypeString`,
  `TypeHash`, `TypeList`, `TypeSet`, `TypeZSet`, `TypeStream`, `TypeJson`, `Ok`, `Warn`, `Danger`,
  `SurfaceDetached`, `OpenRow`), not 22 — the enum is `#[non_exhaustive]`, so this task's palette
  data structure should account for new tokens being addable later without breaking existing
  theme files, but the starting count for parity-checking the new data-driven implementation
  against the old hard-coded one is 21.
- **`crates/app/src/main.rs` builds the theme with `Theme::new(terminal::detect_color_depth())`**
  — no theme selection, no config read for it, no flag.
- **`crates/core/src/config/mod.rs`'s config structs use `#[serde(deny_unknown_fields)]`** on (at
  least) the two top-level structs checked. Adding `theme: Option<String>` and
  `themes: Option<HashMap<...>>` fields is additive for a *new* binary reading an *old* config file
  (missing optional fields are fine), but an *old* binary reading a *new* config file that sets
  `theme`/`themes` will reject it outright — `deny_unknown_fields` is exactly what makes a typo'd
  `passwordEnv` fail loudly (R1.5's whole point), and it has the same effect on a forward-compatible
  field it has never heard of. This is a real deployment consideration (a user who upgrades, sets a
  theme, then needs to roll back the binary gets a config parse failure) worth flagging rather than
  silently accepting.
- **DESIGN §5's role table names `accent` and `surface`/`surface-alt` as semantic tokens.** The
  actual `Token` enum has neither literally — `SurfaceDetached` exists (background-only, documented
  as DESIGN's `surface-alt`'s "first real use") but there is no `Surface`/`Accent` token; `Selected`
  plays the role DESIGN calls `accent` (selection, cursor row). This is a pre-existing naming gap
  between the design doc and the code, not something this task is required to fix, but worth
  resolving — or at minimum documenting as intentional — while this task is already touching every
  token's definition.
- **DESIGN §5 states `local` is "neutral."** The actual `true_color` match gives `Token::EnvLocal`
  `Color::Rgb(0x7A, 0xC7, 0x8E)` — a green, the same hue family as `Token::Ok`. This is a real
  divergence from the design doc worth a decision during this task, since a light/high-contrast
  palette has to decide the same four Environment colours fresh anyway: either bring the code in
  line with "neutral" (a grey/muted tone, freeing green exclusively for `Ok`), or update DESIGN §5
  to describe what shipped. **Flagged as a decision to confirm at build time**, not resolved here.
- **`crates/app/src/terminal.rs`'s `resolve_color_depth`** treats `NO_COLOR` as "monochrome if set
  to any value, including an empty string" (`env.no_color.is_some()`). That is a deviation from
  the spec, not a reading of it. no-color.org, checked 2026-10-01: software "should check for a
  `NO_COLOR` environment variable that, when present **and not an empty string** (regardless of
  its value), prevents the addition of ANSI color." The parenthesis is about non-empty values
  (`NO_COLOR=0` still disables colour); an empty `NO_COLOR=` must not.

## Decisions

1. **`Theme` becomes data: a `Token → Style`-per-`ColorDepth` palette**, not three hard-coded match
   statements. Recommended shape: `Theme { depth: ColorDepth, palette: Palette }` where `Palette`
   holds per-depth colour maps (or per-depth palettes resolved once at load time to a flat
   `HashMap<Token, Style>` or a fixed-size array indexed by `Token as usize` — **confirm at build
   time** which; a fixed array is more in the spirit of "columnar, not a `HashMap`," ADR-0010's
   discipline extended by analogy, and `Token`'s `#[non_exhaustive]` status means the array needs a
   documented "new token defaults to X if a theme doesn't define it" fallback rule).
2. **Built-ins: dark (today's exact palette, becoming the first data-driven instance rather than a
   literal match), light, and high-contrast.** Each checked for WCAG AA contrast (4.5:1 for normal
   text, per WCAG 2.1) over every token pair DESIGN §8 cares about — text-on-surface, each
   `env.*`/`type.*` foreground against the pane background, `Selected`'s fixed dark-on-amber pair.
   A test (not a human eye-check) computes contrast ratios and fails below 4.5:1 for any checked
   pair in any built-in theme at any `ColorDepth` where the comparison is meaningful (monochrome has
   no literal colour contrast to check — the existing rule that monochrome carries meaning via
   glyph/weight, not hue, stands).
3. **Theme selection: `--theme` flag > config `theme` field > dark default**, following the same
   "flags beat everything" precedence R1.2/R1.10 already establish for connection resolution — this
   task should reuse that precedence shape rather than invent a new one. The config field is
   additive (`theme: Option<String>`, defaulting to the dark built-in when absent), consistent with
   `deny_unknown_fields`'s existing behavior for every other optional field.
4. **User themes as config data**: `themes: { "<name>": { "<tokenName>": "#rrggbb", ... } }`, a
   token→colour map per named theme, selected the same way a built-in is (`--theme mytheme` or
   config `theme: "mytheme"`). **Unknown token names are a parse error** — consistent with
   `deny_unknown_fields`'s existing philosophy that a typo (`"bordr-focus"` instead of
   `"border-focus"`) should fail loudly, not be silently ignored, the same reasoning R1.5 already
   applies to `passwordEnv`. **Partial maps inherit from a named base** (`base: "dark"` or similar,
   defaulting to the dark built-in) — so a user theme only needs to override the handful of tokens
   they actually want to change, not restate all 21.
5. **The `deny_unknown_fields` forward-compatibility gap is accepted, not solved, in this task** —
   flagged in Context above as a real but pre-existing class of problem (every additive config field
   has this property; `theme`/`themes` are not special), **confirm at build time** whether it is
   worth a one-line callout in the config's own doc/error message or left as the existing, accepted
   trade-off R1.5/ADR-0002 already made for every other field.
6. **`NO_COLOR` follows the spec** (see Context): `resolve_color_depth` changes
   `env.no_color.is_some()` to `env.no_color.as_deref().is_some_and(|v| !v.is_empty())`. Unit tests
   pin both halves — `NO_COLOR=""` leaves colour on, `NO_COLOR=0` turns it off — since a one-line
   change like this is easy to silently revert without them.
7. **The `accent`/`surface` naming gap and the `EnvLocal`-is-green-not-neutral divergence** —
   **both confirmed as decisions for build time**, not resolved by this doc. Recommended default if
   no stronger preference emerges: keep `Token`'s existing names (`Selected`, `SurfaceDetached`)
   rather than renaming to match DESIGN's prose exactly (a rename is pure churn across every
   render-side call site for no behavioural change), and update DESIGN §5's role table to name the
   tokens that actually exist; for `EnvLocal`, lean toward bringing the code in line with "neutral"
   (a muted grey, closer to `EnvUnknown`'s treatment but still visually distinct from it) since the
   design intent — "local is not a safety signal the way staging/prod are" — reads as the more
   deliberate statement, but this should be confirmed with the user before three new palettes all
   bake in a color choice that might be reverted.

## Architecture

All core-side: `Theme`, `Palette`, the WCAG contrast check, and the token→colour parsing all live
in `crates/core/src/theme/` and `crates/core/src/config/` — pure data and pure functions, no I/O,
golden-frame-testable exactly as `ColorDepth` degradation already is (M0 task 3's proof: "golden
frames of one screen in all three modes," now extended to "...in all three modes, times three
themes" for the two new built-ins). Shell-side: `crates/app/src/main.rs` reads the `--theme` flag
and the config's resolved theme, applying the precedence in decision 3, and constructs `Theme`
once at startup exactly as it does today with `Theme::new(detect_color_depth())` — the shell's role
here is purely "gather inputs and hand them to the pure constructor," no new runtime behavior.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/theme/mod.rs` | `Theme`/`Palette` become data-driven; three built-in palettes (dark, light, high-contrast); WCAG contrast check function + test |
| `crates/core/src/config/mod.rs` | `theme: Option<String>`, `themes: Option<HashMap<String, ThemeOverride>>` config fields; parse-time validation (unknown token name is a parse error; partial-map inheritance) |
| `crates/app/src/main.rs` | `--theme` flag parsing; theme-selection precedence (flag > config > dark default) |
| `crates/app/src/terminal.rs` | `resolve_color_depth`'s `NO_COLOR` handling, per decision 6 |
| `docs/DESIGN.md` §5 | record the three built-ins (today's doc only promises two) and, per decision 7, either the renamed/clarified token table or an explicit note on the `accent`/`surface` naming gap |
| `crates/core/tests/golden.rs` | light and high-contrast goldens for at least one representative screen, per ADR-0011's stated promise |

## Testing

- **Core unit tests**: WCAG AA contrast holds for every checked token pair in all three built-ins,
  at `TrueColor` (and `Ansi256` where the degraded palette differs meaningfully); theme selection
  precedence (flag/config/default) resolves correctly across all combinations; a user theme with a
  typo'd token name fails to parse with a clear error naming the bad key (same shape as an
  unknown-field config error elsewhere); a partial user-theme map inherits every token it does not
  override from its named base; the `NO_COLOR` empty-value test from decision 6.
- **Golden frames**: at least one full-screen frame (the keys pane, per existing precedent) in
  light and high-contrast, at `TrueColor`, fulfilling ADR-0011's "light-theme goldens" promise
  explicitly named in PLAN's row.
- **A by-hand script**, per the planning doc's own verification note: `--theme light` driven
  against `scripts/fixtures.py`-seeded data, to catch anything a contrast *number* passes but a
  human eye still finds wrong (e.g. a hue that is technically AA-compliant but ugly or confusing).

## CLAUDE.md rules this binds

- **Colors are semantic tokens, never literals.** This task is the first time that rule is tested
  against more than one palette — if any render-side code has snuck in a literal `Color::Rgb(...)`
  outside `theme/mod.rs`, this task's second and third palettes will immediately expose it (a
  literal color does not change when the theme does), which is itself a useful audit this task
  should run early (grep for `Color::Rgb`/`Color::Indexed` outside `theme/mod.rs` before writing
  the new palettes, to catch any existing violation rather than assume there is none).
- **Terminal capability degrades gracefully.** Each of the three built-ins needs its own
  `Ansi256`/`Monochrome` degradation, not just a `TrueColor` definition — monochrome in particular
  must still satisfy "colour never carries meaning alone" for all three themes identically, since
  monochrome by definition strips the one thing that would otherwise distinguish them.
- **Config... Unknown fields are a parse error.** Directly extended by decision 4's unknown-token
  rule for user themes — same philosophy, new surface.

## Out of scope

- **Terminal background auto-detection (OSC 11)** — explicitly out of scope for M4 per the task
  brief; theme/background matching is a real future nicety, not asked for here.
- **A theme picker UI** (browsing/previewing themes from inside the app) — selection is a config
  field and a flag, same mechanism as everything else resolved at launch; no in-app theme switcher,
  consistent with CLAUDE.md's "screen space is a budget" and the project's general preference for
  config-driven rather than UI-driven settings.
- **Renaming `Token` variants to match DESIGN's `accent`/`surface` prose exactly** — flagged as a
  decision (7) but the recommended default is not to rename; if that default is overridden, the
  rename becomes its own, larger mechanical change across every render call site.

## Outcome

Built as planned, with these build-time resolutions and deviations:

- **Shape:** `Palette` holds a fixed array per depth indexed by `Token::index()`; a token a palette
  does not set sets nothing (the terminal's colours show through). `Theme` resolves it once into a
  `[Style; Token::COUNT]` and stays `Copy`. `Theme::new(depth)` is still `dark`.
- **Token names** in user themes are DESIGN's: `text`, `muted`, `border`, `border-focus`,
  `selected`, `env.*`, `type.*`, `ok`, `warn`, `danger`, `surface-detached`. `open-row` is not
  themable (a modifier) and is refused with a message saying so. Values are `"#rrggbb"` or
  `{ "fg", "bg" }`; `selected` needs the object form to set its foreground.
- **256 colours** for `light`, `high-contrast` and user themes is the nearest cube/grey index;
  `dark`'s are the original hand-picked ones. Light's `warn`/`env.staging`/`type.hash` are an
  olive-brown (`#6B5A00`) and `type.zset` a brown-orange, because the 256-colour grid has no
  amber dark enough for AA on white that is also distinct from the zset hue.
- **Dark changed in two ways:** `env.local` (decided), and `muted` (`#8A8A8A` → `#8C8C8C`; 256:
  245 → 248, and `env.unknown` 246 → 247). The contrast test found `muted` at 4.49:1 on the
  detached wash, and the 256-colour wash is lighter still. Only goldens containing `muted`
  changed.
- **Config validation:** `theme` must name a built-in or a user theme, a user theme's `base` must be
  a built-in, and a user theme may not reuse a built-in's name — each a `ConfigError`. An unknown
  `--theme` exits with the config exit code, listing what is available.
- **Rollback gap:** accepted; one note in the `Config::theme` doc and DESIGN §5.
- **Not done:** the by-hand `--theme light` run against seeded data (needs a TTY and Redis).


## Follow-up: themes paint their own background

Status: **built (2026-10-04).** Build-time resolutions are under Outcome below.

### Why

By-hand testing found `--theme light` unreadable on a dark terminal: key names, column headers
and the title bar's target are near-black on near-black. That is the design as written ("the app
does not paint the terminal background, so it cannot make either work on the wrong one"), but the
design has two defects:

1. **The contrast test certifies a background nobody sees.** `Palette::background` is what every
   foreground is measured against, and it is never painted. On a real terminal the test passes
   whatever the reader actually sees.
2. **The failure is silent.** Nothing says "this theme wants a light terminal"; the most natural
   first thing anyone tries, `--theme light`, looks broken.

Chosen (user decision, over OSC 11 detection and over documenting the limitation): **a theme may
paint its own background.**

### Decisions

1. **A `background` token.** A new themable token whose value is the colour the app paints behind
   everything it draws. `light` and `high-contrast` set it to the background they are already
   designed against (`Palette::background`), so on those themes what the contrast test measures is
   what is on screen. **`dark` does not set it**: the default keeps blending with the reader's
   terminal, and `main`'s look is unchanged — no `dark` golden may change.
2. **Painted at every depth that has colour.** Truecolor and 256 (quantized like every other
   slot). **Monochrome paints nothing** — no theme varies there, consistent with the rest of §5.
3. **Painted everywhere the app draws, with no holes.** The whole frame first; then every overlay
   (help, confirm dialogs, add forms, notifications, the INFO overlay — anything that uses
   ratatui's `Clear` or resets a cell) must re-paint the background rather than reset to the
   terminal default, or a light theme shows dark rectangles. `Selected` and `surface-detached`
   keep their own backgrounds on top. Find every `Clear`/`Color::Reset`/`Style::reset()` in
   `crates/core/src/render` and decide each one.
4. **User themes.** `background` is settable like any token (`"#rrggbb"` — a bare value sets the
   background, as for `selected`/`surface-detached`). A user theme inherits its base's: `base:
   light` paints, `base: dark` does not. To *not* paint on a painting base, `"background":
   "terminal"` (the terminal's own shows through). Any other non-colour value is a parse error.
5. **The contrast test measures against the painted background when there is one**, and against
   `Palette::background` (the assumed terminal) only for a theme that does not paint. Document in
   `Palette::background`'s doc comment which it now is.
6. **`dark` stays transparent on purpose**, and that is the one remaining case where the contrast
   claim rests on an assumed terminal background. Say so in DESIGN §5.

### Testing

- Goldens: light and high-contrast golden frames now show the painted background in every cell
  (re-blessing *those* is expected); add one with an overlay open (help or a confirm dialog) in
  `light`, proving the overlay has no unpainted hole. **Every `dark` golden must be unchanged.**
- A unit/golden test that in `light` no cell of a rendered frame (including under an overlay) has
  a default/reset background at truecolor and at 256.
- Monochrome: unchanged for every theme.
- Config: `background` parses as a colour; `"terminal"` parses and disables painting on a light
  base; a typo'd value is a parse error with line/column.

### Docs

DESIGN §5 (themes paragraph, token table gains `background`, the "app does not paint" sentence is
replaced), this doc's Outcome, and the CLAUDE.md "Colors are semantic tokens" bullet if it needs it.

### Outcome of the follow-up

Built as decided, with these build-time resolutions:

- **One post-pass, not a pre-fill.** `render::paint_background` runs last in `frame()`, after the
  final overlay. A pre-filled background cannot survive: `put` replaces a cell's style
  (`Style::reset()` first), and overlays blank their rectangles with spaces written through it, so
  each would reset the cell to the terminal's. Painting once at the end means no draw path can
  leave a hole and none has to remember to avoid one. It touches only cells whose background is
  still the terminal default, so `Selected` and `surface-detached` keep theirs, and it returns
  before touching the buffer when `Theme::background()` is `None` — which is why no `dark` golden
  changed.
- **Foreground too.** A cell showing text with no foreground gets `text`'s. Not in the spec, but a
  default foreground is light on a dark terminal, and light-on-light is the original bug again.
  Blank cells are left with a default foreground.
- **`Clear` / `Color::Reset` / `Style::reset()` audit** over `crates/core/src/render`: there is no
  ratatui `Clear` and no `Color::Reset` written anywhere. The only `Style::reset()` is in `put`,
  which every overlay uses to blank and draw; it is deliberate (it stops a border's DIM or a
  Selected background bleeding into text drawn over it) and is left as is, the post-pass
  repainting what it resets. `Color::Reset` appears only in `is_plain`/`to_golden`, which read it.
  Direct `set_style`/`set_symbol` writes (selection fill, editor area, the reversed-cursor repaint)
  set their own colours and are unaffected.
- **Shape:** `Token::Background` (`"background"`), appended last so existing indices are stable.
  `Palette::painted_background(depth)` is the slot; `Palette::ground(depth)` is that or the
  assumed `Palette::background`, and is what the contrast test measures. `Theme::background()`
  is the resolved colour. `ColorSpec::Terminal` clears the slot; the config parser accepts
  `"terminal"` for `background` only, via a seed that knows which token it is reading.
- **256 colours:** the painted background is quantised like every slot (`#FFFFFF` is 231,
  `#000000` is 16).
- **Goldens re-blessed:** `browser_style_light_truecolor`, `browser_style_light_ansi256` and
  `browser_style_high_contrast_truecolor` (styles and map only; the text is unchanged). New:
  `help_overlay_light_truecolor`. No `dark` or monochrome golden changed.
