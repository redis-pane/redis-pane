//! The Dashboard view (`g d`, R6.3, M3 task 6, `docs/plans/m3-dashboard.md`):
//! the tile grid, its breakpoints (decision 5, recorded in DESIGN.md §2),
//! and the raw-`INFO` overlay (decision 6).
//!
//! Mirrors the Slowlog/Monitor/Pub-Sub views' own shape — a summary line,
//! a body, drawn into the area `render::frame` hands over — but the body is
//! a grid of tiles rather than a list, so it gets its own layout function
//! rather than reusing `layout::layout` (that one only ever knows the
//! two-pane Keys/Value split, per its own doc comment).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::clock::Clock;
use crate::glyphs::{Glyph, Glyphs};
use crate::state::State;
use crate::state::dashboard::{AlarmLevel, MemoryTile, TILE_ORDER, TileId, tiles_per_row};
use crate::state::scan::thousands;
use crate::state::value::plural;
use crate::theme::{Theme, Token};

use super::keys::{Viewport, truncate};
use super::{put, put_right};

/// One tile's box: a title line (top border), two content lines, a bottom
/// border — four rows tall. A one-row gap separates one grid row of tiles
/// from the next, so `TILE_STEP` is what the viewport math below actually
/// scrolls by.
const TILE_HEIGHT: u16 = 4;
const TILE_STEP: u16 = TILE_HEIGHT + 1;

/// Render the Dashboard view into `area` — the whole body between the
/// title bar and the status/hint bars, exactly as `frame` hands it over.
pub fn render(state: &State, theme: &Theme, clock: &dyn Clock, area: Rect, buf: &mut Buffer) {
    if area.width < 8 || area.height < 3 {
        return;
    }
    let mut y = area.y;
    summary_line(state, theme, clock, area, y, buf);
    y += 1;

    let Some(raw) = state.dashboard.raw() else {
        // Nothing has landed yet — "no blank frame" (decision 1) is met by
        // `g d` issuing a fetch immediately, but the frame drawn *before*
        // that reply lands still has to say something rather than leave the
        // body empty, the same reasoning `slowlog::empty_state` follows.
        let (text, token) = if let Some(detail) = &state.dashboard.error {
            (
                format!("{} INFO failed: {detail}", theme.glyphs.get(Glyph::Deleted)),
                Token::Danger,
            )
        } else {
            (theme.glyphs.text("⟳ fetching…").into_owned(), Token::Muted)
        };
        put(
            buf,
            area.x + 1,
            y,
            &truncate(&text, area.width.saturating_sub(2) as usize, theme.glyphs),
            theme.style(token),
        );
        return;
    };
    let _ = raw; // tiles below read through `state.dashboard`'s own accessors.

    // A stale-error line, reserved only while there both is an error *and*
    // there is last-good data to show underneath it (decision 7: the last
    // good values stay on screen with their age, and an error nobody read
    // is an error nobody handled) — the same "reserved only when true" rule
    // the keys pane's cap banner and Slowlog's own summary line follow.
    if let Some(detail) = &state.dashboard.error {
        put(
            buf,
            area.x + 1,
            y,
            &truncate(
                &format!(
                    "{} INFO failed: {detail} {} showing the last good reading",
                    theme.glyphs.get(Glyph::Deleted),
                    theme.glyphs.get(Glyph::Dash)
                ),
                area.width.saturating_sub(2) as usize,
                theme.glyphs,
            ),
            theme.style(Token::Danger),
        );
        y += 1;
    }

    if let Some(overlay) = state.dashboard.expanded_tile {
        overlay_box(state, theme, overlay, area, y, buf);
        return;
    }

    grid(state, theme, area, y, buf);
}

fn summary_line(
    state: &State,
    theme: &Theme,
    clock: &dyn Clock,
    area: Rect,
    y: u16,
    buf: &mut Buffer,
) {
    let age = state.dashboard.last_updated_ms.map(|at| {
        let secs = clock.now_epoch_ms().saturating_sub(at) / 1000;
        format!("updated {}s ago", secs)
    });
    let text = match &age {
        Some(age) => format!("DASHBOARD {} {age}", theme.glyphs.get(Glyph::Separator)),
        None => "DASHBOARD".to_string(),
    };
    put(buf, area.x + 1, y, &text, theme.style(Token::Text));
    if state.dashboard.loading {
        put_right(
            buf,
            area.x,
            y,
            area.width.saturating_sub(1),
            &theme.glyphs.text("⟳ fetching…"),
            theme.style(Token::Muted),
        );
    }
}

/// The grid layout: `tiles_per_row(area.width)` columns (decision 5), rows
/// scrolled — stateless, the same trick `slowlog::render` uses for its own
/// list — so that the focused tile is always on screen without
/// `DashboardState` needing a persisted grid-scroll field of its own.
fn grid(state: &State, theme: &Theme, area: Rect, top: u16, buf: &mut Buffer) {
    let body_height = area.y + area.height - top;
    if body_height == 0 {
        return;
    }
    let cols = tiles_per_row(area.width).max(1);
    let visible_tile_rows = ((body_height / TILE_STEP) as usize).max(1);
    let focused_index = TILE_ORDER
        .iter()
        .position(|&t| t == state.dashboard.focused_tile)
        .unwrap_or(0);
    let focused_row = focused_index / cols;
    let viewport = Viewport {
        offset: 0,
        selected: focused_row,
    }
    .scrolled_to_selection(visible_tile_rows);

    let col_w = (area.width as usize).saturating_sub(cols.saturating_sub(1)) / cols;
    let col_w = col_w.max(10) as u16;

    for (i, &tile) in TILE_ORDER.iter().enumerate() {
        let row = i / cols;
        if row < viewport.offset || row >= viewport.offset + visible_tile_rows {
            continue;
        }
        let col = i % cols;
        let x = area.x + col as u16 * (col_w + 1);
        let y = top + (row - viewport.offset) as u16 * TILE_STEP;
        if x + col_w > area.x + area.width {
            continue;
        }
        draw_tile(state, theme, tile, x, y, col_w, buf);
    }
}

/// Bytes, rendered the way a human reads them — the same ladder
/// `keys::format_size` uses, widened to `u64` and one more unit: a
/// `used_memory` figure routinely exceeds the `u32` bytes a key's own size
/// column is capped at (ADR-0010's cap is on the *Loaded set*, not on a
/// server's total memory).
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// A bar, `width` characters wide, filled to `ratio` (0.0–1.0) — the memory
/// tile's own reading of "as a bar" (decision 3, DESIGN's own mock). The full
/// and empty cells are the [`Glyph::BarFull`]/[`Glyph::BarEmpty`] roles
/// (`█`/`░`, or `#`/`-` in the ASCII set), the same one glyph vocabulary the
/// sparkline below reads its levels from.
fn bar(ratio: f64, width: usize, g: Glyphs) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    format!(
        "{}{}",
        g.get(Glyph::BarFull).repeat(filled),
        g.get(Glyph::BarEmpty).repeat(width - filled)
    )
}

/// A sparkline over the ops/sec history (decision 3) — eight levels, read from
/// [`Glyphs::sparkline_ladder`]: the Unicode block-element ladder `▁▂▃▄▅▆▇█`,
/// or `_.:-=+*#` in the ASCII set. One column per level in both.
fn sparkline(values: &[u64], width: usize, g: Glyphs) -> String {
    let levels = g.sparkline_ladder();
    if width == 0 || values.is_empty() {
        return String::new();
    }
    let tail: Vec<u64> = values.iter().rev().take(width).rev().copied().collect();
    let max = tail.iter().copied().max().unwrap_or(0).max(1);
    tail.iter()
        .map(|&v| {
            let level = ((v as f64 / max as f64) * (levels.len() - 1) as f64).round() as usize;
            levels[level.min(levels.len() - 1)]
        })
        .collect::<String>()
}

/// The token an [`AlarmLevel`] paints with — semantic tokens only, never a
/// literal colour (CLAUDE.md).
fn alarm_token(level: AlarmLevel) -> Token {
    match level {
        AlarmLevel::Ok => Token::Ok,
        AlarmLevel::Warn => Token::Warn,
        AlarmLevel::Danger => Token::Danger,
    }
}

fn draw_tile(state: &State, theme: &Theme, tile: TileId, x: u16, y: u16, w: u16, buf: &mut Buffer) {
    let focused = state.dashboard.focused_tile == tile;
    let level = tile_level(state, tile);
    let border_token = if focused {
        Token::BorderFocus
    } else {
        Token::Border
    };
    let border = theme.style(border_token);
    let iw = w.saturating_sub(2) as usize; // inner width, inside the box's side borders

    // Top border, with the tile's own title in it — the same shape
    // `draw_confirm_box`'s frame uses, one box among several instead of one
    // centred overlay. Focus is marked *inside* the title, not by
    // overwriting the box's corner glyph: `>` for the focused tile, a space
    // for every other one, so the title starts at the same column and
    // nothing in the box shifts between the two states — only the border's
    // own colour token (below) and this marker carry the difference, and
    // the marker keeps it legible in monochrome where colour carries
    // nothing (CLAUDE.md: "colour never carries meaning alone").
    put(
        buf,
        x,
        y,
        &format!(
            "{}{}{}",
            theme.glyphs.get(Glyph::CornerTopLeft),
            theme
                .glyphs
                .get(Glyph::Horizontal)
                .repeat(w.saturating_sub(2) as usize),
            theme.glyphs.get(Glyph::CornerTopRight)
        ),
        border,
    );
    let marker = if focused { ">" } else { " " };
    put(
        buf,
        x + 2,
        y,
        &format!("{marker} {}", tile.label()),
        theme.style(Token::Text),
    );

    let (line1, line2) = tile_lines(state, tile, iw, theme.glyphs);
    let value_token = theme.style(alarm_token(level));
    put(
        buf,
        x + 1,
        y + 1,
        &truncate(&line1, iw, theme.glyphs),
        value_token,
    );
    put(
        buf,
        x + 1,
        y + 2,
        &truncate(&line2, iw, theme.glyphs),
        theme.style(Token::Muted),
    );

    put(
        buf,
        x,
        y + 3,
        &format!(
            "{}{}{}",
            theme.glyphs.get(Glyph::CornerBottomLeft),
            theme
                .glyphs
                .get(Glyph::Horizontal)
                .repeat(w.saturating_sub(2) as usize),
            theme.glyphs.get(Glyph::CornerBottomRight)
        ),
        border,
    );
    // Side borders for the two content rows — the top/bottom `put` calls
    // above only drew the horizontal rules.
    put(buf, x, y + 1, theme.glyphs.get(Glyph::Vertical), border);
    put(
        buf,
        x + w - 1,
        y + 1,
        theme.glyphs.get(Glyph::Vertical),
        border,
    );
    put(buf, x, y + 2, theme.glyphs.get(Glyph::Vertical), border);
    put(
        buf,
        x + w - 1,
        y + 2,
        theme.glyphs.get(Glyph::Vertical),
        border,
    );
}

fn tile_level(state: &State, tile: TileId) -> AlarmLevel {
    match tile {
        TileId::Memory => state
            .dashboard
            .memory_tile()
            .map_or(AlarmLevel::Ok, |t| t.level),
        TileId::HitRatio => state
            .dashboard
            .hit_ratio_tile()
            .map_or(AlarmLevel::Ok, |t| t.level),
        // Ops/sec has no alarm of its own (decision 4 names none) — always
        // painted `Ok` (the neutral token, not a claim that traffic is
        // healthy).
        TileId::Ops => AlarmLevel::Ok,
        TileId::Clients => state
            .dashboard
            .clients_tile()
            .map_or(AlarmLevel::Ok, |t| t.level),
        TileId::Replication => state
            .dashboard
            .replication_tile()
            .map_or(AlarmLevel::Ok, |t| t.level),
        TileId::Eviction => state
            .dashboard
            .eviction_tile()
            .map_or(AlarmLevel::Ok, |t| t.level),
    }
}

/// The two content lines inside one tile's box, `iw` characters wide.
fn tile_lines(state: &State, tile: TileId, iw: usize, g: Glyphs) -> (String, String) {
    let dash = || (g.get(Glyph::Dash).to_string(), String::new());
    match tile {
        TileId::Memory => match state.dashboard.memory_tile() {
            Some(MemoryTile {
                used_bytes,
                peak_bytes,
                max_bytes: Some(max),
                ..
            }) => {
                let ratio = used_bytes as f64 / max as f64;
                (
                    format!(
                        "{} / {} ({:.0}%)",
                        format_bytes(used_bytes),
                        format_bytes(max),
                        ratio * 100.0
                    ),
                    format!(
                        "[{}] peak {}",
                        bar(ratio, iw.saturating_sub(2).clamp(4, 20), g),
                        format_bytes(peak_bytes)
                    ),
                )
            }
            Some(MemoryTile {
                used_bytes,
                peak_bytes,
                max_bytes: None,
                ..
            }) => (
                format!("{} (no limit)", format_bytes(used_bytes)),
                format!("peak {}", format_bytes(peak_bytes)),
            ),
            None => dash(),
        },
        TileId::HitRatio => match state.dashboard.hit_ratio_tile() {
            Some(t) => (
                match t.ratio() {
                    Some(r) => format!("{:.1}% hit rate", r * 100.0),
                    None => "no data yet".to_string(),
                },
                format!(
                    "{} hits / {} misses",
                    thousands(t.hits),
                    thousands(t.misses)
                ),
            ),
            None => dash(),
        },
        TileId::Ops => {
            let history = state.dashboard.ops_history();
            let last = history.back().copied().unwrap_or(0);
            (
                format!("{} ops/sec", thousands(last)),
                // One narrower than `iw`: `truncate` below keeps a column
                // spare, and a sparkline built to the full width lost its
                // newest point — the one that matters — to a `…`.
                sparkline(
                    &history.iter().copied().collect::<Vec<_>>(),
                    iw.saturating_sub(1).min(60),
                    g,
                ),
            )
        }
        TileId::Clients => match state.dashboard.clients_tile() {
            Some(t) => (
                format!("{} connected", thousands(t.connected)),
                format!("{} blocked", t.blocked),
            ),
            None => dash(),
        },
        TileId::Replication => match state.dashboard.replication_tile() {
            Some(t) if t.role == "master" => (
                format!(
                    "{} {} {}",
                    t.role,
                    g.get(Glyph::Separator),
                    plural(t.connected_replicas as usize, "replica")
                ),
                match t.lag_secs {
                    Some(lag) => format!("worst lag {lag}s"),
                    None => "no replicas".to_string(),
                },
            ),
            Some(t) => (
                t.role.clone(),
                match (t.link_status.as_deref(), t.lag_secs) {
                    (Some(status), Some(lag)) => {
                        format!("{status} {} lag {lag}s", g.get(Glyph::Separator))
                    }
                    (Some(status), None) => status.to_string(),
                    (None, Some(lag)) => format!("lag {lag}s"),
                    (None, None) => String::new(),
                },
            ),
            None => dash(),
        },
        TileId::Eviction => match state.dashboard.eviction_tile() {
            Some(t) => (
                format!("evicted {}", thousands(t.evicted_keys)),
                format!("expired {}", thousands(t.expired_keys)),
            ),
            None => dash(),
        },
    }
}

/// The raw-`INFO` overlay (decision 6): the expanded tile's whole section,
/// `key:value` per line, scrollable. Styled like `draw_confirm_box`'s frame
/// (the same border/title shape every overlay in this app uses) but drawn
/// directly rather than through it — that helper has no notion of a scroll
/// offset, which this needs and a confirm dialog never has.
fn overlay_box(state: &State, theme: &Theme, tile: TileId, area: Rect, top: u16, buf: &mut Buffer) {
    let Some((_, fields)) = state.dashboard.expanded_section() else {
        return;
    };
    let h = (area.y + area.height).saturating_sub(top);
    if h < 3 {
        return;
    }
    let w = area.width;
    let border = theme.style(Token::Border);
    put(
        buf,
        area.x,
        top,
        &format!(
            "{}{}{}",
            theme.glyphs.get(Glyph::CornerTopLeft),
            theme
                .glyphs
                .get(Glyph::Horizontal)
                .repeat(w.saturating_sub(2) as usize),
            theme.glyphs.get(Glyph::CornerTopRight)
        ),
        border,
    );
    put(
        buf,
        area.x + 2,
        top,
        &format!(
            " {} {} raw INFO ",
            tile.section_name(),
            theme.glyphs.get(Glyph::Dash)
        ),
        theme.style(Token::Text),
    );
    let content_rows = (h.saturating_sub(2)) as usize;
    let max_scroll = fields.len().saturating_sub(content_rows.max(1));
    let scroll = state.dashboard.overlay_scroll.min(max_scroll);
    for (row, (key, value)) in fields.iter().skip(scroll).take(content_rows).enumerate() {
        let y = top + 1 + row as u16;
        put(buf, area.x, y, theme.glyphs.get(Glyph::Vertical), border);
        let line = format!("{key}:{value}");
        put(
            buf,
            area.x + 2,
            y,
            &truncate(&line, (w.saturating_sub(4)) as usize, theme.glyphs),
            theme.style(Token::Text),
        );
        put(
            buf,
            area.x + w - 1,
            y,
            theme.glyphs.get(Glyph::Vertical),
            border,
        );
    }
    for row in fields.len().saturating_sub(scroll)..content_rows {
        let y = top + 1 + row as u16;
        put(buf, area.x, y, theme.glyphs.get(Glyph::Vertical), border);
        put(
            buf,
            area.x + w - 1,
            y,
            theme.glyphs.get(Glyph::Vertical),
            border,
        );
    }
    put(
        buf,
        area.x,
        top + h - 1,
        &format!(
            "{}{}{}",
            theme.glyphs.get(Glyph::CornerBottomLeft),
            theme
                .glyphs
                .get(Glyph::Horizontal)
                .repeat(w.saturating_sub(2) as usize),
            theme.glyphs.get(Glyph::CornerBottomRight)
        ),
        border,
    );
}
