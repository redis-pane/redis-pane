//! Glyph roles and their Unicode / ASCII variants (DESIGN §5, PLAN M4 task 6).
//!
//! Widgets ask for a [`Glyph`] by *role* — `Deleted`, `Live`, `Expanded` — never
//! for a literal character, the same discipline [`crate::theme::Token`] applies
//! to colour. A [`Glyphs`] resolves a role to whichever variant is active.
//!
//! The two variants of every role occupy **the same number of terminal
//! columns** (one), so choosing ASCII never moves a column, a truncation point
//! or a golden frame's layout. A test enforces it for the whole table.
//!
//! Colour depth ([`crate::theme::ColorDepth`]) and glyph set are independent
//! capabilities a terminal can have in any combination, so this is its own
//! axis, carried alongside the colour depth on the [`crate::theme::Theme`].
//!
//! Text built outside the renderer — a liveness readout, a server condition, a
//! key label — still carries Unicode glyphs, because the core's state does not
//! know how it will be drawn. [`Glyphs::text`] maps every glyph the table
//! knows in such a string; it is applied at the draw site, never to data read
//! from Redis, which is shown as it was stored.

use std::borrow::Cow;

/// Which variant of the table is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphSet {
    /// The designed set: box drawing, geometric shapes, arrows.
    #[default]
    Unicode,
    /// Plain ASCII, for terminals and locales without the Unicode glyphs.
    Ascii,
}

/// A semantic glyph. Never a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Glyph {
    /// `✕` — deleted, disconnected, failed.
    Deleted,
    /// `✗` — a failed check (`json ✗`).
    CheckFail,
    /// `✓` — a passed check (`json ✓`).
    CheckOk,
    /// `●` — live; also the Environment dot and the Open-key mark.
    Live,
    /// `○` — manual.
    Manual,
    /// `✎` — editing.
    Editing,
    /// `⊘` — detached: the Viewer holds a key that is not the Selected key.
    Detached,
    /// `⟳` — a read is in flight.
    Fetching,
    /// `⚠` — a warning.
    Warn,
    /// `▏` — the text cursor in a filter or input.
    Cursor,
    /// `▌` — the active half of a two-field editor.
    ActiveHalf,
    /// `⁎` — a pattern subscription.
    PatternSub,
    /// `§` — a sharded subscription or message (`SSUBSCRIBE`, M5 task 9).
    Sharded,
    /// `▶` — the focused Pub/Sub strip.
    StripFocused,
    /// `⟡` — the unfocused Pub/Sub strip.
    StripIdle,
    /// `▾` — an expanded tree group.
    Expanded,
    /// `▸` — a collapsed tree group.
    Collapsed,
    /// `▲` — the Open key's row is above the viewport.
    OffAbove,
    /// `▼` — the Open key's row is below the viewport.
    OffBelow,
    /// `…` — truncation.
    Ellipsis,
    /// `·` — a separator between facts.
    Separator,
    /// `·` — a metadata cell not yet fetched. Drawn only by role: [`Glyphs::text`]
    /// reads `·` as [`Glyph::Separator`].
    Pending,
    /// `—` — a dash in prose.
    Dash,
    /// `•` — a redacted secret.
    Redacted,
    /// `∞` — a key with no expiry.
    Infinity,
    /// `µ` — microseconds.
    Micro,
    /// `→`
    Right,
    /// `←`
    Left,
    /// `↑`
    Up,
    /// `↓`
    Down,
    /// `⌃` — the Control modifier.
    Ctrl,
    /// `⌥` — the Alt modifier.
    Alt,
    /// `⏎` — the Enter key.
    Return,
    /// `⌫` — the Backspace key.
    Backspace,
    /// `─`
    Horizontal,
    /// `│`
    Vertical,
    /// `┌`
    CornerTopLeft,
    /// `┐`
    CornerTopRight,
    /// `└`
    CornerBottomLeft,
    /// `┘`
    CornerBottomRight,
    /// `├` — a row tied to the pane's rule.
    TeeRight,
    /// `┊` — the dashed divider of a detached Viewer.
    VerticalDashed,
    /// `┈` — the dashed rule of a detached Viewer.
    HorizontalDashed,
    /// `›` — a breadcrumb separator (`cluster › 10.0.0.3:7001`).
    Crumb,
    /// `█` — a filled bar cell.
    BarFull,
    /// `░` — an empty bar cell.
    BarEmpty,
}

impl Glyph {
    /// Every role. Order matters only to [`Glyphs::text`]: where two roles share
    /// a Unicode character, the earlier one is what the character means in prose.
    pub const ALL: [Glyph; 46] = [
        Glyph::Deleted,
        Glyph::CheckFail,
        Glyph::CheckOk,
        Glyph::Live,
        Glyph::Manual,
        Glyph::Editing,
        Glyph::Detached,
        Glyph::Fetching,
        Glyph::Warn,
        Glyph::Cursor,
        Glyph::ActiveHalf,
        Glyph::PatternSub,
        Glyph::Sharded,
        Glyph::StripFocused,
        Glyph::StripIdle,
        Glyph::Expanded,
        Glyph::Collapsed,
        Glyph::OffAbove,
        Glyph::OffBelow,
        Glyph::Ellipsis,
        Glyph::Separator,
        Glyph::Pending,
        Glyph::Dash,
        Glyph::Redacted,
        Glyph::Infinity,
        Glyph::Micro,
        Glyph::Right,
        Glyph::Left,
        Glyph::Up,
        Glyph::Down,
        Glyph::Ctrl,
        Glyph::Alt,
        Glyph::Return,
        Glyph::Backspace,
        Glyph::Horizontal,
        Glyph::Vertical,
        Glyph::CornerTopLeft,
        Glyph::CornerTopRight,
        Glyph::CornerBottomLeft,
        Glyph::CornerBottomRight,
        Glyph::TeeRight,
        Glyph::VerticalDashed,
        Glyph::HorizontalDashed,
        Glyph::Crumb,
        Glyph::BarFull,
        Glyph::BarEmpty,
    ];

    /// The designed character.
    pub const fn unicode(self) -> &'static str {
        match self {
            Glyph::Deleted => "✕",
            Glyph::CheckFail => "✗",
            Glyph::CheckOk => "✓",
            Glyph::Live => "●",
            Glyph::Manual => "○",
            Glyph::Editing => "✎",
            Glyph::Detached => "⊘",
            Glyph::Fetching => "⟳",
            Glyph::Warn => "⚠",
            Glyph::Cursor => "▏",
            Glyph::ActiveHalf => "▌",
            Glyph::PatternSub => "⁎",
            Glyph::Sharded => "§",
            Glyph::StripFocused => "▶",
            Glyph::StripIdle => "⟡",
            Glyph::Expanded => "▾",
            Glyph::Collapsed => "▸",
            Glyph::OffAbove => "▲",
            Glyph::OffBelow => "▼",
            Glyph::Ellipsis => "…",
            Glyph::Separator | Glyph::Pending => "·",
            Glyph::Dash => "—",
            Glyph::Redacted => "•",
            Glyph::Infinity => "∞",
            Glyph::Micro => "µ",
            Glyph::Right => "→",
            Glyph::Left => "←",
            Glyph::Up => "↑",
            Glyph::Down => "↓",
            Glyph::Ctrl => "⌃",
            Glyph::Alt => "⌥",
            Glyph::Return => "⏎",
            Glyph::Backspace => "⌫",
            Glyph::Horizontal => "─",
            Glyph::Vertical => "│",
            Glyph::CornerTopLeft => "┌",
            Glyph::CornerTopRight => "┐",
            Glyph::CornerBottomLeft => "└",
            Glyph::CornerBottomRight => "┘",
            Glyph::TeeRight => "├",
            Glyph::VerticalDashed => "┊",
            Glyph::HorizontalDashed => "┈",
            Glyph::Crumb => "›",
            Glyph::BarFull => "█",
            Glyph::BarEmpty => "░",
        }
    }

    /// The ASCII stand-in: one column, the plainest character that still reads
    /// as the same shape (DESIGN §5 settled `~` for the Ad-hoc marker the same way).
    pub const fn ascii(self) -> &'static str {
        match self {
            Glyph::Deleted | Glyph::CheckFail => "x",
            Glyph::CheckOk | Glyph::Editing => "+",
            Glyph::Live | Glyph::PatternSub | Glyph::Redacted => "*",
            Glyph::Sharded => "#",
            Glyph::Manual | Glyph::StripIdle => "o",
            Glyph::Detached => "/",
            Glyph::Fetching => "@",
            Glyph::Warn => "!",
            Glyph::Cursor | Glyph::ActiveHalf | Glyph::Vertical => "|",
            Glyph::StripFocused | Glyph::Collapsed | Glyph::Right | Glyph::Crumb => ">",
            Glyph::Expanded | Glyph::OffBelow | Glyph::Down => "v",
            Glyph::OffAbove | Glyph::Up | Glyph::Ctrl => "^",
            Glyph::Ellipsis | Glyph::Infinity => "~",
            Glyph::Separator | Glyph::Dash | Glyph::Horizontal | Glyph::BarEmpty => "-",
            Glyph::Pending | Glyph::HorizontalDashed => ".",
            Glyph::Micro => "u",
            Glyph::Left | Glyph::Return | Glyph::Backspace => "<",
            Glyph::Alt => "M",
            Glyph::CornerTopLeft
            | Glyph::CornerTopRight
            | Glyph::CornerBottomLeft
            | Glyph::CornerBottomRight
            | Glyph::TeeRight => "+",
            Glyph::VerticalDashed => ":",
            Glyph::BarFull => "#",
        }
    }
}

/// The sparkline ladder, lowest level first (Unicode block elements).
const SPARK_UNICODE: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
/// The same eight levels in ASCII, still ascending in visual weight.
const SPARK_ASCII: [&str; 8] = ["_", ".", ":", "-", "=", "+", "*", "#"];

/// The active glyph variant, resolving roles to characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Glyphs {
    pub set: GlyphSet,
}

impl Glyphs {
    pub fn new(set: GlyphSet) -> Self {
        Self { set }
    }

    /// The character for a role, in the active variant.
    pub fn get(&self, glyph: Glyph) -> &'static str {
        match self.set {
            GlyphSet::Unicode => glyph.unicode(),
            GlyphSet::Ascii => glyph.ascii(),
        }
    }

    /// The sparkline ladder, lowest level first.
    pub fn sparkline_ladder(&self) -> &'static [&'static str; 8] {
        match self.set {
            GlyphSet::Unicode => &SPARK_UNICODE,
            GlyphSet::Ascii => &SPARK_ASCII,
        }
    }

    /// Map every glyph the table knows in `s` to the active variant.
    ///
    /// For text the renderer did not build — state-side readouts, key labels.
    /// Never call it on a value, a key name or a field read from Redis: that is
    /// data, and shown as stored. In Unicode mode this borrows `s` unchanged.
    pub fn text<'a>(&self, s: &'a str) -> Cow<'a, str> {
        if self.set == GlyphSet::Unicode || s.is_ascii() {
            return Cow::Borrowed(s);
        }
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if c.is_ascii() {
                out.push(c);
                continue;
            }
            let mut buf = [0u8; 4];
            let c_str: &str = c.encode_utf8(&mut buf);
            match Glyph::ALL.iter().find(|g| g.unicode() == c_str) {
                Some(g) => out.push_str(g.ascii()),
                None => out.push(c),
            }
        }
        Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    fn width(s: &str) -> usize {
        Line::from(s).width()
    }

    #[test]
    fn every_role_is_one_column_in_both_variants() {
        for g in Glyph::ALL {
            assert_eq!(width(g.unicode()), 1, "{g:?} unicode {:?}", g.unicode());
            assert_eq!(width(g.ascii()), 1, "{g:?} ascii {:?}", g.ascii());
            assert_eq!(g.unicode().chars().count(), 1, "{g:?}");
            assert_eq!(g.ascii().chars().count(), 1, "{g:?}");
        }
    }

    #[test]
    fn the_sparkline_ladders_are_one_column_per_level() {
        for set in [GlyphSet::Unicode, GlyphSet::Ascii] {
            let ladder = Glyphs::new(set).sparkline_ladder();
            for level in ladder {
                assert_eq!(width(level), 1, "{set:?} {level:?}");
            }
        }
    }

    #[test]
    fn ascii_variants_are_ascii_and_unicode_variants_are_not() {
        for g in Glyph::ALL {
            assert!(g.ascii().is_ascii(), "{g:?}");
            assert!(!g.unicode().is_ascii(), "{g:?}");
        }
        for level in Glyphs::new(GlyphSet::Ascii).sparkline_ladder() {
            assert!(level.is_ascii());
        }
    }

    #[test]
    fn the_ascii_ladder_has_eight_distinct_levels() {
        let ladder = Glyphs::new(GlyphSet::Ascii).sparkline_ladder();
        let mut seen = ladder.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn get_resolves_to_the_active_variant() {
        assert_eq!(Glyphs::new(GlyphSet::Unicode).get(Glyph::Deleted), "✕");
        assert_eq!(Glyphs::new(GlyphSet::Ascii).get(Glyph::Deleted), "x");
    }

    #[test]
    fn text_maps_every_known_glyph_and_keeps_width() {
        let ascii = Glyphs::new(GlyphSet::Ascii);
        let src = "● live · changed 2s ago ⌃K ↑↓ jk ✕ OOM — … ∞ 42µs";
        let out = ascii.text(src);
        assert!(out.is_ascii(), "{out}");
        assert_eq!(width(src), width(&out), "layout must not move");
        assert_eq!(out, "* live - changed 2s ago ^K ^v jk x OOM - ~ ~ 42us");
    }

    #[test]
    fn text_in_unicode_mode_is_untouched() {
        let uni = Glyphs::default();
        assert!(matches!(uni.text("● live · ok"), Cow::Borrowed(_)));
    }

    #[test]
    fn text_leaves_unknown_characters_alone() {
        // Not in the table: data, not chrome.
        assert_eq!(Glyphs::new(GlyphSet::Ascii).text("café ✕"), "café x");
    }
}
