//! Semantic colour tokens and capability degradation (DESIGN §5, PLAN M0.3).
//!
//! Widgets ask for [`Token::BorderFocus`] or [`Token::TypeHash`], never a hex
//! value. Themes remap tokens; the terminal's capability decides how far a
//! token can be honoured. Truecolor degrades to 256 and then to monochrome.
//!
//! The rule that makes monochrome usable: **colour never carries meaning
//! alone**. An Environment is a coloured dot *and* the word `prod`; a type is a
//! coloured cell *and* the word `hash`. Losing colour must lose emphasis, never
//! information.

use ratatui::style::{Color, Modifier, Style};

/// What the terminal can actually display. Detected by the shell, injected like
/// the clock so that golden frames can pin it (ADR-0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Monochrome,
}

/// A semantic colour token. Never a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Token {
    /// Ordinary foreground text.
    Text,
    /// De-emphasised text: units, labels, secondary facts.
    Muted,
    /// Pane borders and separators.
    Border,
    /// The border of the focused pane.
    BorderFocus,
    /// The current selection: a full-row highlight, not a foreground tint.
    /// Used for exactly one row at a time, in the keys pane.
    Selected,
    EnvLocal,
    EnvStaging,
    EnvProd,
    EnvUnknown,
    /// One hue per Redis type — DESIGN §5's `type.*`, consistent everywhere a
    /// type appears: the keys pane's marker and TYPE column, and the value
    /// pane's header. `Other`/binary deliberately has no token of its own; an
    /// unclassified type is neutral, not a ninth colour to keep track of.
    TypeString,
    TypeHash,
    TypeList,
    TypeSet,
    TypeZSet,
    TypeStream,
    TypeJson,
    /// Liveness is healthy.
    Ok,
    /// Something needs attention but nothing is broken.
    Warn,
    /// Destructive, or refused.
    Danger,
    /// The background of a Viewer holding a key that is not the Selected key —
    /// DESIGN §5's `surface-alt`, and its first real use.
    ///
    /// A *background only*: it never sets a foreground, so every token drawn
    /// over it keeps its own hue and the body stays as readable as it was. In
    /// monochrome it is nothing at all, deliberately — the dashed divider, the
    /// header chip and the underlined row are what carry the state where there
    /// is no colour, which is why the wash is never shipped as the only signal.
    SurfaceDetached,
    /// The Open key's row in the keys pane, when it is not the Selected key.
    ///
    /// Ranked deliberately below [`Token::Selected`]: the cursor is a full bar,
    /// this is an underline. Two emphases in one list only work if one of them
    /// is obviously the junior, and underline is the one modifier still free in
    /// monochrome once the selection has taken reverse video.
    OpenRow,
}

impl Token {
    /// How many tokens there are — the length of a palette's arrays.
    pub const COUNT: usize = 21;

    /// Every token, in the order a palette stores them.
    pub const ALL: [Token; Token::COUNT] = [
        Token::Text,
        Token::Muted,
        Token::Border,
        Token::BorderFocus,
        Token::Selected,
        Token::EnvLocal,
        Token::EnvStaging,
        Token::EnvProd,
        Token::EnvUnknown,
        Token::TypeString,
        Token::TypeHash,
        Token::TypeList,
        Token::TypeSet,
        Token::TypeZSet,
        Token::TypeStream,
        Token::TypeJson,
        Token::Ok,
        Token::Warn,
        Token::Danger,
        Token::SurfaceDetached,
        Token::OpenRow,
    ];

    /// The token's slot in a palette's arrays — its position in [`Token::ALL`].
    pub fn index(self) -> usize {
        Token::ALL
            .iter()
            .position(|t| *t == self)
            .expect("Token::ALL lists every token")
    }

    /// The name a user theme spells the token with (DESIGN §5's role names).
    /// `OpenRow` has none: it is an underline, not a colour, and no theme
    /// restyles it.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            Token::Text => "text",
            Token::Muted => "muted",
            Token::Border => "border",
            Token::BorderFocus => "border-focus",
            Token::Selected => "selected",
            Token::EnvLocal => "env.local",
            Token::EnvStaging => "env.staging",
            Token::EnvProd => "env.prod",
            Token::EnvUnknown => "env.unknown",
            Token::TypeString => "type.string",
            Token::TypeHash => "type.hash",
            Token::TypeList => "type.list",
            Token::TypeSet => "type.set",
            Token::TypeZSet => "type.zset",
            Token::TypeStream => "type.stream",
            Token::TypeJson => "type.json",
            Token::Ok => "ok",
            Token::Warn => "warn",
            Token::Danger => "danger",
            Token::SurfaceDetached => "surface-detached",
            Token::OpenRow => return None,
        })
    }

    /// The inverse of [`Token::name`].
    pub fn from_name(name: &str) -> Option<Token> {
        Token::ALL.into_iter().find(|t| t.name() == Some(name))
    }

    /// The channel a bare `"#rrggbb"` in a user theme sets: the background for
    /// the two tokens that are backgrounds, the foreground for the rest.
    fn primary_is_background(self) -> bool {
        matches!(self, Token::Selected | Token::SurfaceDetached)
    }
}

/// A 24-bit colour. The one place a literal colour is allowed to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Parse `#rrggbb`. Nothing else: no names, no `#rgb`, no alpha.
    pub fn parse(text: &str) -> Option<Rgb> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(digits, 16).ok().map(Rgb::hex)
    }

    fn color(self) -> Color {
        Color::Rgb(self.0, self.1, self.2)
    }

    /// WCAG 2.1 relative luminance.
    pub fn luminance(self) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.0) + 0.7152 * lin(self.1) + 0.0722 * lin(self.2)
    }
}

/// WCAG 2.1 contrast ratio between two colours: 1.0 (identical) to 21.0.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// What a 256-colour index looks like, for contrast checking and for
/// quantising a user's colour. Indices 0–15 are whatever the terminal says
/// they are; the standard xterm values are assumed.
pub fn xterm_rgb(i: u8) -> Rgb {
    const BASE: [u32; 16] = [
        0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xC0C0C0, 0x808080,
        0xFF0000, 0x00FF00, 0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
    ];
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match i {
        0..=15 => Rgb::hex(BASE[usize::from(i)]),
        16..=231 => {
            let n = usize::from(i - 16);
            Rgb(LEVELS[n / 36], LEVELS[n / 6 % 6], LEVELS[n % 6])
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            Rgb(v, v, v)
        }
    }
}

/// The nearest cube or greyscale index (16–255) to a colour. The sixteen
/// system colours are skipped: a terminal may have remapped them to anything.
fn quantize(c: Rgb) -> u8 {
    let dist = |p: Rgb| {
        let d = |a: u8, b: u8| i32::from(a).abs_diff(i32::from(b)).pow(2);
        d(p.0, c.0) + d(p.1, c.1) + d(p.2, c.2)
    };
    (16..=255u8)
        .min_by_key(|i| dist(xterm_rgb(*i)))
        .expect("16..=255 is not empty")
}

/// What one token sets: a foreground, a background, either or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot<C> {
    pub fg: Option<C>,
    pub bg: Option<C>,
}

impl<C> Slot<C> {
    /// Sets nothing: the terminal's own colours show through.
    const NONE: Slot<C> = Slot { fg: None, bg: None };

    fn fg(c: C) -> Self {
        Slot {
            fg: Some(c),
            bg: None,
        }
    }

    fn bg(c: C) -> Self {
        Slot {
            fg: None,
            bg: Some(c),
        }
    }

    fn pair(fg: C, bg: C) -> Self {
        Slot {
            fg: Some(fg),
            bg: Some(bg),
        }
    }
}

/// A colour a user theme gives one token: a bare `"#rrggbb"` sets the token's
/// primary channel, an `{ "fg": .., "bg": .. }` sets the channels it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpec {
    Primary(Rgb),
    Channels { fg: Option<Rgb>, bg: Option<Rgb> },
}

/// A theme's colours: for every token, what it sets at truecolor and at 256
/// colours. Monochrome is not themed — it is hue-free by definition, so every
/// theme degrades to the same thing (see [`Theme`]).
///
/// A token a palette does not define sets nothing, so the terminal's own
/// colours show through: a token added later never breaks an older theme, it
/// is merely unstyled in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub name: String,
    /// The terminal background this palette was designed against. Never
    /// painted — the app does not set the terminal's background — but it is
    /// what the contrast test measures every foreground against, and why
    /// `light` is for light terminals.
    pub background: Rgb,
    true_color: [Slot<Rgb>; Token::COUNT],
    ansi_256: [Slot<u8>; Token::COUNT],
}

fn quantize_slot(s: Slot<Rgb>) -> Slot<u8> {
    Slot {
        fg: s.fg.map(quantize),
        bg: s.bg.map(quantize),
    }
}

impl Palette {
    /// Both depths given explicitly: `dark`'s 256 colours are hand-picked.
    fn explicit(
        name: &str,
        background: Rgb,
        true_color: &[(Token, Slot<Rgb>)],
        ansi_256: &[(Token, Slot<u8>)],
    ) -> Self {
        let mut p = Palette {
            name: name.into(),
            background,
            true_color: [Slot::NONE; Token::COUNT],
            ansi_256: [Slot::NONE; Token::COUNT],
        };
        for (t, s) in true_color {
            p.true_color[t.index()] = *s;
        }
        for (t, s) in ansi_256 {
            p.ansi_256[t.index()] = *s;
        }
        p
    }

    /// Truecolor given; 256 colours are the nearest cube/grey index.
    fn quantized(name: &str, background: Rgb, true_color: &[(Token, Slot<Rgb>)]) -> Self {
        let mut p = Palette::explicit(name, background, true_color, &[]);
        for i in 0..Token::COUNT {
            p.ansi_256[i] = quantize_slot(p.true_color[i]);
        }
        p
    }

    /// `dark` — the original palette, unchanged except that `env.local` is a
    /// neutral grey rather than a green (DESIGN §5: green is for `ok` alone).
    pub fn dark() -> Self {
        use Token::*;
        let h = |v| Slot::fg(Rgb::hex(v));
        Palette::explicit(
            "dark",
            Rgb::hex(0x1A1B26),
            &[
                (Text, h(0xE6E6E6)),
                (Muted, h(0x8C8C8C)),
                (Border, h(0x444444)),
                (BorderFocus, h(0x7AA2F7)),
                // A full bar, not a tint: a fixed dark-on-amber pair that
                // reads clearly regardless of which type or metadata colour
                // it covers.
                (Selected, Slot::pair(Rgb::hex(0x11131A), Rgb::hex(0xF5A742))),
                (EnvLocal, h(0xB0B0B0)),
                (EnvStaging, h(0xE0AF68)),
                (EnvProd, h(0xF7768E)),
                (EnvUnknown, h(0x9A9A9A)),
                (TypeString, h(0x7DCFFF)),
                (TypeHash, h(0xE0AF68)),
                (TypeList, h(0x9ECE6A)),
                (TypeSet, h(0xBB9AF7)),
                (TypeZSet, h(0xFF9E64)),
                (TypeStream, h(0xF7768E)),
                (TypeJson, h(0x73DACA)),
                (Ok, h(0x7AC78E)),
                (Warn, h(0xE0AF68)),
                (Danger, h(0xF7768E)),
                // Dark enough to sit under body text without touching
                // contrast, and blue enough to read as a state rather than as
                // a highlight — the amber of `Selected` is spoken for, and
                // the two must never be mistaken for each other.
                (SurfaceDetached, Slot::bg(Rgb::hex(0x1E2433))),
            ],
            &[
                (Text, Slot::fg(253)),
                (Muted, Slot::fg(248)),
                (Border, Slot::fg(238)),
                (BorderFocus, Slot::fg(111)),
                (Selected, Slot::pair(0, 208)),
                (EnvLocal, Slot::fg(249)),
                (EnvStaging, Slot::fg(179)),
                (EnvProd, Slot::fg(210)),
                (EnvUnknown, Slot::fg(247)),
                (TypeString, Slot::fg(117)),
                (TypeHash, Slot::fg(179)),
                (TypeList, Slot::fg(150)),
                (TypeSet, Slot::fg(141)),
                (TypeZSet, Slot::fg(215)),
                (TypeStream, Slot::fg(210)),
                (TypeJson, Slot::fg(116)),
                (Ok, Slot::fg(114)),
                (Warn, Slot::fg(179)),
                (Danger, Slot::fg(210)),
                (SurfaceDetached, Slot::bg(236)),
            ],
        )
    }

    /// `light` — for a light terminal background.
    pub fn light() -> Self {
        use Token::*;
        let h = |v| Slot::fg(Rgb::hex(v));
        Palette::quantized(
            "light",
            Rgb::hex(0xFFFFFF),
            &[
                (Text, h(0x1F2328)),
                (Muted, h(0x57606A)),
                (Border, h(0xBFC5CC)),
                (BorderFocus, h(0x0550AE)),
                (Selected, Slot::pair(Rgb::hex(0x11131A), Rgb::hex(0xF5A742))),
                (EnvLocal, h(0x2F3A46)),
                (EnvStaging, h(0x6B5A00)),
                (EnvProd, h(0xB4162B)),
                (EnvUnknown, h(0x666666)),
                (TypeString, h(0x0B6E99)),
                (TypeHash, h(0x6B5A00)),
                (TypeList, h(0x166B2E)),
                (TypeSet, h(0x6639BA)),
                (TypeZSet, h(0x8A5A00)),
                (TypeStream, h(0xB4162B)),
                (TypeJson, h(0x00695C)),
                (Ok, h(0x166B2E)),
                (Warn, h(0x6B5A00)),
                (Danger, h(0xB4162B)),
                (SurfaceDetached, Slot::bg(Rgb::hex(0xE6ECF7))),
            ],
        )
    }

    /// `high-contrast` — pure black ground, saturated foregrounds, every
    /// foreground at AAA (7:1) rather than AA.
    pub fn high_contrast() -> Self {
        use Token::*;
        let h = |v| Slot::fg(Rgb::hex(v));
        Palette::quantized(
            "high-contrast",
            Rgb::hex(0x000000),
            &[
                (Text, h(0xFFFFFF)),
                (Muted, h(0xCCCCCC)),
                (Border, h(0x999999)),
                (BorderFocus, h(0x00FFFF)),
                (Selected, Slot::pair(Rgb::hex(0x000000), Rgb::hex(0xFFD700))),
                (EnvLocal, h(0xE0E0E0)),
                (EnvStaging, h(0xFFD700)),
                (EnvProd, h(0xFF8080)),
                (EnvUnknown, h(0xAAAAAA)),
                (TypeString, h(0x5FD7FF)),
                (TypeHash, h(0xFFD700)),
                (TypeList, h(0x87FF5F)),
                (TypeSet, h(0xD7AFFF)),
                (TypeZSet, h(0xFFAF5F)),
                (TypeStream, h(0xFF80A8)),
                (TypeJson, h(0x5FFFD7)),
                (Ok, h(0x5FFF5F)),
                (Warn, h(0xFFD700)),
                (Danger, h(0xFF8080)),
                (SurfaceDetached, Slot::bg(Rgb::hex(0x10264D))),
            ],
        )
    }

    /// A built-in by name.
    pub fn builtin(name: &str) -> Option<Palette> {
        match name {
            "dark" => Some(Palette::dark()),
            "light" => Some(Palette::light()),
            "high-contrast" => Some(Palette::high_contrast()),
            _ => None,
        }
    }

    /// Give one token a colour. The 256-colour slot follows by quantising, so a
    /// user theme needs only one colour per token.
    pub fn set(&mut self, token: Token, spec: ColorSpec) {
        let i = token.index();
        let mut slot = self.true_color[i];
        match spec {
            ColorSpec::Primary(c) if token.primary_is_background() => slot.bg = Some(c),
            ColorSpec::Primary(c) => slot.fg = Some(c),
            ColorSpec::Channels { fg, bg } => {
                slot.fg = fg.or(slot.fg);
                slot.bg = bg.or(slot.bg);
            }
        }
        self.true_color[i] = slot;
        self.ansi_256[i] = quantize_slot(slot);
    }

    /// What `token` sets at truecolor.
    pub fn true_color(&self, token: Token) -> Slot<Rgb> {
        self.true_color[token.index()]
    }

    /// What `token` sets at 256 colours.
    pub fn ansi_256(&self, token: Token) -> Slot<u8> {
        self.ansi_256[token.index()]
    }
}

/// The built-in theme names, for messages.
pub const BUILTIN_NAMES: [&str; 3] = ["dark", "light", "high-contrast"];

/// Why a theme could not be chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTheme {
    pub name: String,
    pub available: Vec<String>,
}

impl std::fmt::Display for UnknownTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown theme \"{}\" (available: {})",
            self.name,
            self.available.join(", ")
        )
    }
}

impl std::error::Error for UnknownTheme {}

/// Choose the palette: `--theme` beats the config's `theme`, which beats
/// `dark` — the same "flags beat everything" order as connection resolution
/// (R1.2). A user theme is built on its `base` (default `dark`).
pub fn select(
    flag: Option<&str>,
    config: Option<&crate::config::Config>,
) -> Result<Palette, UnknownTheme> {
    let name = flag
        .or(config.and_then(|c| c.theme.as_deref()))
        .unwrap_or("dark");
    if let Some(def) = config.and_then(|c| c.themes.get(name)) {
        // `config::parse` has validated the base; fall back to dark rather
        // than panic for a hand-built `Config`.
        let mut p = def
            .base
            .as_deref()
            .and_then(Palette::builtin)
            .unwrap_or_else(Palette::dark);
        p.name = name.to_string();
        for (token, spec) in &def.colors {
            p.set(*token, *spec);
        }
        return Ok(p);
    }
    Palette::builtin(name).ok_or_else(|| UnknownTheme {
        name: name.to_string(),
        available: BUILTIN_NAMES
            .iter()
            .map(|s| s.to_string())
            .chain(config.into_iter().flat_map(|c| c.themes.keys().cloned()))
            .collect(),
    })
}

/// Resolves tokens to styles at a given colour depth.
///
/// Built once at startup from a [`Palette`] and a depth; `style` is an array
/// lookup. It stays `Copy` so a frame can take it by reference as freely as
/// before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub depth: ColorDepth,
    styles: [Style; Token::COUNT],
}

impl Theme {
    /// The `dark` theme at `depth`.
    pub fn new(depth: ColorDepth) -> Self {
        Theme::with_palette(depth, &Palette::dark())
    }

    pub fn with_palette(depth: ColorDepth, palette: &Palette) -> Self {
        let mut styles = [Style::default(); Token::COUNT];
        for token in Token::ALL {
            let i = token.index();
            let mut style = match depth {
                ColorDepth::TrueColor => {
                    let s = palette.true_color[i];
                    paint(s.fg.map(Rgb::color), s.bg.map(Rgb::color))
                }
                ColorDepth::Ansi256 => {
                    let s = palette.ansi_256[i];
                    paint(s.fg.map(Color::Indexed), s.bg.map(Color::Indexed))
                }
                ColorDepth::Monochrome => monochrome(token),
            };
            if token == Token::OpenRow {
                // Underline, so the Open key's row stays distinguishable from
                // the cursor's reverse video with no hue at all — and at every
                // depth, in every theme.
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            styles[i] = style;
        }
        Theme { depth, styles }
    }

    /// The style for a token, degraded to what the terminal can show.
    ///
    /// In monochrome, hue is gone entirely and the only tools left are bold,
    /// dim and reverse video — which is why every token that carries meaning
    /// is also spelled out in words, or in the case of the selection, in
    /// position, somewhere on screen.
    pub fn style(&self, token: Token) -> Style {
        self.styles[token.index()]
    }
}

fn paint(fg: Option<Color>, bg: Option<Color>) -> Style {
    let mut s = Style::new();
    if let Some(c) = fg {
        s = s.fg(c);
    }
    if let Some(c) = bg {
        s = s.bg(c);
    }
    s
}

/// Monochrome is the same in every theme: no hue, so nothing to theme.
fn monochrome(token: Token) -> Style {
    let s = Style::default();
    match token {
        // Reverse video is the one signal that survives with no hue at all —
        // the standard way a terminal marks "this is the row you are on" when
        // it cannot colour it.
        Token::Selected => s.add_modifier(Modifier::REVERSED),
        // The wash is hue and nothing else, so here it is nothing. Making it
        // DIM instead would collide with the focus signal, which already owns
        // intensity on the Viewer's header row.
        Token::SurfaceDetached => s,
        Token::Muted | Token::Border | Token::EnvUnknown => s.add_modifier(Modifier::DIM),
        Token::BorderFocus | Token::EnvProd | Token::Danger => s.add_modifier(Modifier::BOLD),
        // Every type token: no modifier. The TYPE column's word is what
        // carries the information here, per this module's own rule.
        _ => s,
    }
}

/// The token that stands for an Environment.
pub fn env_token(env: crate::state::Environment) -> Token {
    use crate::state::Environment as E;
    match env {
        E::Local => Token::EnvLocal,
        E::Staging => Token::EnvStaging,
        E::Prod => Token::EnvProd,
        E::Unknown => Token::EnvUnknown,
    }
}

/// The token for a Redis type, wherever one is shown (DESIGN §5's `type.*`).
/// `None` — metadata not yet fetched — is [`Token::Muted`], matching every
/// other pending cell.
pub fn type_token(kind: Option<crate::state::KeyKind>) -> Token {
    use crate::state::KeyKind as K;
    match kind {
        None => Token::Muted,
        Some(K::String) => Token::TypeString,
        Some(K::Hash) => Token::TypeHash,
        Some(K::List) => Token::TypeList,
        Some(K::Set) => Token::TypeSet,
        Some(K::ZSet) => Token::TypeZSet,
        Some(K::Stream) => Token::TypeStream,
        Some(K::Json) => Token::TypeJson,
        // Unclassified is neutral, not a ninth colour to keep track of.
        Some(K::Other) => Token::Muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::KeyKind;

    #[test]
    fn truecolor_and_256_differ_but_both_carry_hue() {
        let t = Theme::new(ColorDepth::TrueColor).style(Token::EnvProd);
        let a = Theme::new(ColorDepth::Ansi256).style(Token::EnvProd);
        assert_ne!(t, a);
        assert!(matches!(t.fg, Some(Color::Rgb(..))));
        assert!(matches!(a.fg, Some(Color::Indexed(_))));
    }

    #[test]
    fn monochrome_sets_no_foreground_colour_at_all() {
        for token in [Token::EnvProd, Token::EnvLocal, Token::Ok, Token::Danger] {
            assert_eq!(Theme::new(ColorDepth::Monochrome).style(token).fg, None);
        }
    }

    #[test]
    fn prod_stays_emphasised_when_hue_is_gone() {
        let s = Theme::new(ColorDepth::Monochrome).style(Token::EnvProd);
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn every_type_gets_a_distinct_hue_in_truecolor() {
        let theme = Theme::new(ColorDepth::TrueColor);
        let kinds = [
            KeyKind::String,
            KeyKind::Hash,
            KeyKind::List,
            KeyKind::Set,
            KeyKind::ZSet,
            KeyKind::Stream,
            KeyKind::Json,
        ];
        let colors: Vec<_> = kinds
            .iter()
            .map(|k| theme.style(type_token(Some(*k))).fg)
            .collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(
                    colors[i], colors[j],
                    "{:?} and {:?} share a hue",
                    kinds[i], kinds[j]
                );
            }
        }
    }

    #[test]
    fn a_type_not_yet_fetched_is_muted_like_every_other_pending_cell() {
        assert_eq!(type_token(None), Token::Muted);
    }

    #[test]
    fn an_unclassified_type_is_neutral_not_a_ninth_colour() {
        assert_eq!(type_token(Some(KeyKind::Other)), Token::Muted);
    }

    #[test]
    fn selection_is_a_full_bar_with_both_a_background_and_a_foreground() {
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
            let s = Theme::new(depth).style(Token::Selected);
            assert!(s.bg.is_some(), "{depth:?} selection has no background");
            assert!(s.fg.is_some(), "{depth:?} selection has no foreground");
        }
    }

    #[test]
    fn selection_in_monochrome_is_reverse_video_with_no_colour_at_all() {
        let s = Theme::new(ColorDepth::Monochrome).style(Token::Selected);
        assert!(s.add_modifier.contains(Modifier::REVERSED));
        assert!(s.fg.is_none());
        assert!(s.bg.is_none());
    }

    // ── M4 task 5: themes are data ──────────────────────────────────────────

    fn builtins() -> Vec<Palette> {
        BUILTIN_NAMES
            .iter()
            .map(|n| Palette::builtin(n).unwrap())
            .collect()
    }

    /// The foreground tokens text is drawn in. `Border` is a decorative rule,
    /// not text, and is deliberately quiet; everything else must be readable.
    const READABLE: [Token; 17] = [
        Token::Text,
        Token::Muted,
        Token::BorderFocus,
        Token::EnvLocal,
        Token::EnvStaging,
        Token::EnvProd,
        Token::EnvUnknown,
        Token::TypeString,
        Token::TypeHash,
        Token::TypeList,
        Token::TypeSet,
        Token::TypeZSet,
        Token::TypeStream,
        Token::TypeJson,
        Token::Ok,
        Token::Warn,
        Token::Danger,
    ];

    const AA: f64 = 4.5;
    const AAA: f64 = 7.0;

    fn fg_rgb(p: &Palette, depth: ColorDepth, t: Token) -> Rgb {
        match depth {
            ColorDepth::TrueColor => p.true_color(t).fg.unwrap(),
            _ => xterm_rgb(p.ansi_256(t).fg.unwrap()),
        }
    }

    fn bg_rgb(p: &Palette, depth: ColorDepth, t: Token) -> Rgb {
        match depth {
            ColorDepth::TrueColor => p.true_color(t).bg.unwrap(),
            _ => xterm_rgb(p.ansi_256(t).bg.unwrap()),
        }
    }

    #[test]
    fn wcag_ratio_matches_the_published_extremes() {
        let black = Rgb(0, 0, 0);
        let white = Rgb(255, 255, 255);
        assert!((contrast_ratio(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 1e-9);
        // #767676 on white is the well-known smallest grey that passes AA.
        assert!(contrast_ratio(Rgb(0x76, 0x76, 0x76), white) >= AA);
        assert!(contrast_ratio(Rgb(0x77, 0x77, 0x77), white) < AA);
    }

    #[test]
    fn every_builtin_meets_wcag_aa_at_truecolor_and_256() {
        for p in builtins() {
            for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                let floor = if p.name == "high-contrast" { AAA } else { AA };
                for t in READABLE {
                    let fg = fg_rgb(&p, depth, t);
                    let over_ground = contrast_ratio(fg, p.background);
                    assert!(
                        over_ground >= floor,
                        "{} {depth:?} {t:?} on the ground: {over_ground:.2}",
                        p.name
                    );
                    let wash = bg_rgb(&p, depth, Token::SurfaceDetached);
                    let over_wash = contrast_ratio(fg, wash);
                    assert!(
                        over_wash >= AA,
                        "{} {depth:?} {t:?} on the detached wash: {over_wash:.2}",
                        p.name
                    );
                }
                let bar = contrast_ratio(
                    fg_rgb(&p, depth, Token::Selected),
                    bg_rgb(&p, depth, Token::Selected),
                );
                assert!(bar >= floor, "{} {depth:?} Selected bar: {bar:.2}", p.name);
            }
        }
    }

    #[test]
    fn every_builtin_defines_every_colour_token_at_both_colour_depths() {
        for p in builtins() {
            for t in Token::ALL {
                if t == Token::OpenRow {
                    continue;
                }
                let (tc, c256) = (p.true_color(t), p.ansi_256(t));
                assert!(
                    tc.fg.is_some() || tc.bg.is_some(),
                    "{} leaves {t:?} unset at truecolor",
                    p.name
                );
                assert!(
                    c256.fg.is_some() || c256.bg.is_some(),
                    "{} leaves {t:?} unset at 256",
                    p.name
                );
            }
        }
    }

    #[test]
    fn dark_is_todays_palette_except_that_local_is_no_longer_green() {
        let t = Theme::new(ColorDepth::TrueColor);
        assert_eq!(t.style(Token::Text).fg, Some(Color::Rgb(0xE6, 0xE6, 0xE6)));
        assert_eq!(t.style(Token::Ok).fg, Some(Color::Rgb(0x7A, 0xC7, 0x8E)));
        assert_eq!(
            t.style(Token::Selected).bg,
            Some(Color::Rgb(0xF5, 0xA7, 0x42))
        );
        let a = Theme::new(ColorDepth::Ansi256);
        assert_eq!(a.style(Token::Selected).bg, Some(Color::Indexed(208)));
        assert_eq!(a.style(Token::Ok).fg, Some(Color::Indexed(114)));
    }

    #[test]
    fn env_local_is_a_neutral_grey_distinct_from_unknown_and_from_ok_in_every_theme() {
        for p in builtins() {
            let Rgb(r, g, b) = p.true_color(Token::EnvLocal).fg.unwrap();
            let spread = r.max(g).max(b) - r.min(g).min(b);
            assert!(spread <= 0x20, "{} env.local is not neutral", p.name);
            assert_ne!(
                p.true_color(Token::EnvLocal),
                p.true_color(Token::EnvUnknown),
                "{}",
                p.name
            );
            assert_ne!(
                p.true_color(Token::EnvLocal).fg,
                p.true_color(Token::Ok).fg,
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn every_type_has_its_own_hue_in_every_theme_at_both_colour_depths() {
        let kinds = [
            Token::TypeString,
            Token::TypeHash,
            Token::TypeList,
            Token::TypeSet,
            Token::TypeZSet,
            Token::TypeStream,
            Token::TypeJson,
        ];
        for p in builtins() {
            for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                let theme = Theme::with_palette(depth, &p);
                for i in 0..kinds.len() {
                    for j in (i + 1)..kinds.len() {
                        assert_ne!(
                            theme.style(kinds[i]).fg,
                            theme.style(kinds[j]).fg,
                            "{} {depth:?}: {:?} and {:?} share a hue",
                            p.name,
                            kinds[i],
                            kinds[j]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn monochrome_is_identical_in_every_theme() {
        let dark = Theme::new(ColorDepth::Monochrome);
        for p in builtins() {
            assert_eq!(Theme::with_palette(ColorDepth::Monochrome, &p), dark);
        }
    }

    #[test]
    fn the_open_row_underline_survives_every_theme_and_depth() {
        for p in builtins() {
            for depth in [
                ColorDepth::TrueColor,
                ColorDepth::Ansi256,
                ColorDepth::Monochrome,
            ] {
                let s = Theme::with_palette(depth, &p).style(Token::OpenRow);
                assert!(s.add_modifier.contains(Modifier::UNDERLINED));
            }
        }
    }

    #[test]
    fn token_names_round_trip_and_open_row_has_none() {
        for t in Token::ALL {
            match t.name() {
                Some(n) => assert_eq!(Token::from_name(n), Some(t)),
                None => assert_eq!(t, Token::OpenRow),
            }
            assert_eq!(Token::ALL[t.index()], t);
        }
        assert_eq!(Token::from_name("bordr-focus"), None);
    }

    #[test]
    fn hex_colours_are_strict() {
        assert_eq!(Rgb::parse("#0a1B2c"), Some(Rgb(0x0A, 0x1B, 0x2C)));
        for bad in [
            "", "#", "#fff", "0a1b2c", "#0a1b2", "#0a1b2cd", "#gggggg", "red",
        ] {
            assert_eq!(Rgb::parse(bad), None, "{bad}");
        }
    }

    fn config(text: &str) -> crate::config::Config {
        crate::config::parse(text).unwrap()
    }

    #[test]
    fn selection_is_flag_then_config_then_dark() {
        let cfg = config(r#"{"theme":"light"}"#);
        let empty = config("{}");
        let name = |flag, cfg| select(flag, cfg).unwrap().name;
        assert_eq!(name(None, None), "dark");
        assert_eq!(name(None, Some(&empty)), "dark");
        assert_eq!(name(None, Some(&cfg)), "light");
        assert_eq!(name(Some("high-contrast"), Some(&cfg)), "high-contrast");
        assert_eq!(name(Some("dark"), Some(&cfg)), "dark");
    }

    #[test]
    fn a_flag_naming_no_theme_says_what_is_available() {
        let cfg = config(r##"{"themes":{"mine":{}}}"##);
        let err = select(Some("nope"), Some(&cfg)).unwrap_err();
        let msg = err.to_string();
        for want in ["nope", "dark", "light", "high-contrast", "mine"] {
            assert!(msg.contains(want), "{msg}");
        }
    }

    #[test]
    fn a_user_theme_overrides_only_what_it_names_and_inherits_the_rest() {
        let cfg = config(
            r##"{"theme":"mine","themes":{"mine":{"base":"light","border-focus":"#ff0000","selected":{"bg":"#00ff00"}}}}"##,
        );
        let p = select(None, Some(&cfg)).unwrap();
        let light = Palette::light();
        assert_eq!(p.true_color(Token::BorderFocus).fg, Some(Rgb(255, 0, 0)));
        // A bare colour on a background token, or a bg-only spec, leaves the
        // other channel as the base had it.
        assert_eq!(p.true_color(Token::Selected).bg, Some(Rgb(0, 255, 0)));
        assert_eq!(
            p.true_color(Token::Selected).fg,
            light.true_color(Token::Selected).fg
        );
        for t in Token::ALL {
            if t != Token::BorderFocus && t != Token::Selected {
                assert_eq!(p.true_color(t), light.true_color(t), "{t:?} not inherited");
                assert_eq!(p.ansi_256(t), light.ansi_256(t), "{t:?} not inherited");
            }
        }
        // The 256-colour slot follows the override.
        assert_ne!(
            p.ansi_256(Token::BorderFocus),
            light.ansi_256(Token::BorderFocus)
        );
    }

    #[test]
    fn a_user_theme_without_a_base_inherits_dark() {
        let cfg = config(r##"{"theme":"mine","themes":{"mine":{"text":"#123456"}}}"##);
        let p = select(None, Some(&cfg)).unwrap();
        assert_eq!(p.true_color(Token::Text).fg, Some(Rgb(0x12, 0x34, 0x56)));
        assert_eq!(
            p.true_color(Token::Muted),
            Palette::dark().true_color(Token::Muted)
        );
    }

    #[test]
    fn quantising_lands_on_the_nearest_cube_or_grey_index() {
        assert_eq!(
            xterm_rgb(quantize(Rgb(0xFF, 0xFF, 0xFF))),
            Rgb(255, 255, 255)
        );
        assert_eq!(
            xterm_rgb(quantize(Rgb(0x80, 0x80, 0x80))),
            Rgb(128, 128, 128)
        );
        assert!(
            quantize(Rgb(0, 0, 0)) >= 16,
            "the system colours are never chosen"
        );
    }
}
