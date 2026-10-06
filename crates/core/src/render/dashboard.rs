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
use crate::state::cluster_dash::{ClusterNode, ClusterTiles, NodeRole};
use crate::state::dashboard::{
    AlarmLevel, DashboardState, MemoryTile, TILE_ORDER, TileId, tiles_per_row,
};
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
    // The node whose tiles are showing: the drilled-in one on a Cluster
    // (M5 task 7), otherwise the Dashboard's own state — so a node view is the
    // single-node Dashboard, drawn by the same code.
    let d = state.dashboard.active();
    let mut y = area.y;
    summary_line(state, theme, clock, area, y, buf);
    y += 1;

    if state.on_cluster()
        && !state.dashboard.drilled()
        && let Some(cluster) = state.dashboard.cluster.as_deref()
    {
        if let Some(detail) = &state.dashboard.error {
            y += error_banner(theme, area, y, detail, buf);
        }
        cluster_overview(cluster, theme, area, y, buf);
        return;
    }

    if d.raw().is_none() {
        // Nothing has landed yet — "no blank frame" (decision 1) is met by
        // `g d` issuing a fetch immediately, but the frame drawn *before*
        // that reply lands still has to say something rather than leave the
        // body empty, the same reasoning `slowlog::empty_state` follows.
        let (text, token) = if let Some(detail) = &state.dashboard.error {
            (
                format!("{} INFO failed: {detail}", theme.glyphs.get(Glyph::Deleted)),
                Token::Danger,
            )
        } else if let Some(detail) = &d.error {
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
    }

    // A stale-error line, reserved only while there both is an error *and*
    // there is last-good data to show underneath it (decision 7: the last
    // good values stay on screen with their age, and an error nobody read
    // is an error nobody handled) — the same "reserved only when true" rule
    // the keys pane's cap banner and Slowlog's own summary line follow.
    if let Some(detail) = d.error.as_ref().or(state.dashboard.error.as_ref()) {
        y += error_banner(theme, area, y, detail, buf);
    }

    if let Some(overlay) = d.expanded_tile {
        overlay_box(d, theme, overlay, area, y, buf);
        return;
    }

    grid(&Tiles::Node(d), Some(d.focused_tile), theme, area, y, buf);
}

/// The stale-error line: `INFO failed: … — showing the last good reading`.
/// Returns the rows it took (always one).
fn error_banner(theme: &Theme, area: Rect, y: u16, detail: &str, buf: &mut Buffer) -> u16 {
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
    1
}

fn summary_line(
    state: &State,
    theme: &Theme,
    clock: &dyn Clock,
    area: Rect,
    y: u16,
    buf: &mut Buffer,
) {
    let age = state.dashboard.active().last_updated_ms.map(|at| {
        let secs = clock.now_epoch_ms().saturating_sub(at) / 1000;
        format!("updated {}s ago", secs)
    });
    let sep = theme.glyphs.get(Glyph::Separator);
    // Inside a node on a Cluster the title carries the way back:
    // `DASHBOARD · cluster › 10.0.0.3:7001 · updated 2s ago` (decision 4).
    let title = match state
        .dashboard
        .cluster
        .as_deref()
        .and_then(|c| c.drilled_node())
    {
        Some(node) => format!(
            "DASHBOARD {sep} cluster {} {}",
            theme.glyphs.get(Glyph::Crumb),
            node.addr
        ),
        None => "DASHBOARD".to_string(),
    };
    let text = match &age {
        Some(age) => format!("{title} {sep} {age}"),
        None => title,
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

/// What the tile grid draws: one node's own `INFO` (the single-node Dashboard,
/// and a node view on a Cluster), or the Cluster's aggregate.
enum Tiles<'a> {
    Node(&'a DashboardState),
    Cluster(&'a ClusterTiles, &'a std::collections::VecDeque<u64>),
}

/// The grid layout: `tiles_per_row(area.width)` columns (decision 5), rows
/// scrolled — stateless, the same trick `slowlog::render` uses for its own
/// list — so that the focused tile is always on screen without
/// `DashboardState` needing a persisted grid-scroll field of its own.
/// `focused` is `None` for the Cluster overview, whose tiles are not
/// individually focusable.
fn grid(
    tiles: &Tiles<'_>,
    focused: Option<TileId>,
    theme: &Theme,
    area: Rect,
    top: u16,
    buf: &mut Buffer,
) {
    let body_height = area.y + area.height - top;
    if body_height == 0 {
        return;
    }
    let cols = tiles_per_row(area.width).max(1);
    let visible_tile_rows = ((body_height / TILE_STEP) as usize).max(1);
    let focused_index = focused
        .and_then(|f| TILE_ORDER.iter().position(|&t| t == f))
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
        draw_tile(tiles, focused == Some(tile), theme, tile, x, y, col_w, buf);
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

#[allow(clippy::too_many_arguments)]
fn draw_tile(
    tiles: &Tiles<'_>,
    focused: bool,
    theme: &Theme,
    tile: TileId,
    x: u16,
    y: u16,
    w: u16,
    buf: &mut Buffer,
) {
    let level = tile_level(tiles, tile);
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

    let (line1, line2) = tile_lines(tiles, tile, iw, theme.glyphs);
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

fn tile_level(tiles: &Tiles<'_>, tile: TileId) -> AlarmLevel {
    match tiles {
        Tiles::Node(d) => match tile {
            TileId::Memory => d.memory_tile().map_or(AlarmLevel::Ok, |t| t.level),
            TileId::HitRatio => d.hit_ratio_tile().map_or(AlarmLevel::Ok, |t| t.level),
            // Ops/sec has no alarm of its own (decision 4 names none) — always
            // painted `Ok` (the neutral token, not a claim that traffic is
            // healthy).
            TileId::Ops => AlarmLevel::Ok,
            TileId::Clients => d.clients_tile().map_or(AlarmLevel::Ok, |t| t.level),
            TileId::Replication => d.replication_tile().map_or(AlarmLevel::Ok, |t| t.level),
            TileId::Eviction => d.eviction_tile().map_or(AlarmLevel::Ok, |t| t.level),
        },
        Tiles::Cluster(t, _) => match tile {
            TileId::Memory => t.memory.level,
            TileId::HitRatio => t.hit_ratio.level,
            TileId::Ops => AlarmLevel::Ok,
            TileId::Clients => t.clients.level,
            TileId::Replication => t.replication.level,
            TileId::Eviction => t.eviction.level,
        },
    }
}

fn hit_ratio_lines(t: crate::state::HitRatioTile) -> (String, String) {
    (
        match t.ratio() {
            Some(r) => format!("{:.1}% hit rate", r * 100.0),
            None => "no data yet".to_string(),
        },
        format!(
            "{} hits / {} misses",
            thousands(t.hits),
            thousands(t.misses)
        ),
    )
}

fn clients_lines(t: crate::state::ClientsTile) -> (String, String) {
    (
        format!("{} connected", thousands(t.connected)),
        format!("{} blocked", t.blocked),
    )
}

fn eviction_lines(t: crate::state::EvictionTile) -> (String, String) {
    (
        format!("evicted {}", thousands(t.evicted_keys)),
        format!("expired {}", thousands(t.expired_keys)),
    )
}

fn ops_lines(
    last: u64,
    history: &std::collections::VecDeque<u64>,
    iw: usize,
    g: Glyphs,
) -> (String, String) {
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

/// The two content lines inside one tile's box, `iw` characters wide.
fn tile_lines(tiles: &Tiles<'_>, tile: TileId, iw: usize, g: Glyphs) -> (String, String) {
    let dash = || (g.get(Glyph::Dash).to_string(), String::new());
    let state = match tiles {
        Tiles::Node(d) => d,
        Tiles::Cluster(t, history) => return cluster_tile_lines(t, history, tile, iw, g),
    };
    match tile {
        TileId::Memory => match state.memory_tile() {
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
        TileId::HitRatio => state.hit_ratio_tile().map_or_else(dash, hit_ratio_lines),
        TileId::Ops => ops_lines(
            state.ops_history().back().copied().unwrap_or(0),
            state.ops_history(),
            iw,
            g,
        ),
        TileId::Clients => state.clients_tile().map_or_else(dash, clients_lines),
        TileId::Replication => match state.replication_tile() {
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
        TileId::Eviction => state.eviction_tile().map_or_else(dash, eviction_lines),
    }
}

/// The Cluster overview's tile lines (M5 task 7, decision 2): the same shapes
/// as a node's, over the aggregate. Memory says "no limit on N nodes" when any
/// primary has no `maxmemory`; Replication is the worst lag across the nodes.
fn cluster_tile_lines(
    t: &ClusterTiles,
    history: &std::collections::VecDeque<u64>,
    tile: TileId,
    iw: usize,
    g: Glyphs,
) -> (String, String) {
    match tile {
        TileId::Memory => {
            let m = &t.memory;
            match m.max_bytes {
                Some(max) => {
                    let ratio = m.used_bytes as f64 / max as f64;
                    (
                        format!(
                            "{} / {} ({:.0}%)",
                            format_bytes(m.used_bytes),
                            format_bytes(max),
                            ratio * 100.0
                        ),
                        format!(
                            "[{}] peak {}",
                            bar(ratio, iw.saturating_sub(2).clamp(4, 20), g),
                            format_bytes(m.peak_bytes)
                        ),
                    )
                }
                None => (
                    format!(
                        "{} (no limit on {})",
                        format_bytes(m.used_bytes),
                        plural(m.no_limit_nodes.max(1), "node")
                    ),
                    format!("peak {}", format_bytes(m.peak_bytes)),
                ),
            }
        }
        TileId::HitRatio => hit_ratio_lines(t.hit_ratio),
        TileId::Ops => ops_lines(t.ops_per_sec, history, iw, g),
        TileId::Clients => clients_lines(t.clients),
        TileId::Replication => {
            let r = &t.replication;
            (
                if r.replicas == 0 {
                    "no replicas".to_string()
                } else {
                    plural(r.replicas, "replica")
                },
                match (r.worst_lag_secs, r.links_down) {
                    (Some(lag), 0) => format!("worst lag {lag}s"),
                    (Some(lag), down) => {
                        format!("worst lag {lag}s {} {down} down", g.get(Glyph::Separator))
                    }
                    (None, 0) => String::new(),
                    (None, down) => format!("{down} down"),
                },
            )
        }
        TileId::Eviction => eviction_lines(t.eviction),
    }
}

/// The raw-`INFO` overlay (decision 6): the expanded tile's whole section,
/// `key:value` per line, scrollable. Styled like `draw_confirm_box`'s frame
/// (the same border/title shape every overlay in this app uses) but drawn
/// directly rather than through it — that helper has no notion of a scroll
/// offset, which this needs and a confirm dialog never has.
fn overlay_box(
    d: &DashboardState,
    theme: &Theme,
    tile: TileId,
    area: Rect,
    top: u16,
    buf: &mut Buffer,
) {
    let Some((_, fields)) = d.expanded_section() else {
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
    let scroll = d.overlay_scroll.min(max_scroll);
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

// ── The Cluster overview (M5 task 7) ─────────────────────────────────────────

/// Rows the node table is always given (header and `MIN_TABLE_NODES` nodes),
/// however short the terminal: the tiles above it never shrink, so when they
/// would not leave this much room the lowest whole tile rows are left off
/// instead, and the table scrolls inside what is left.
const MIN_TABLE_NODES: usize = 3;

/// The node table's columns, in the order they are dropped as the width runs
/// out: lag, then ops/sec, then memory %. Below [`NARROW`] columns only
/// address and role remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TableCols {
    slots: bool,
    memory: bool,
    ops: bool,
    lag: bool,
}

/// Under this many columns the table is address and role only (decision 5).
const NARROW: u16 = 70;

const ROLE_W: usize = 9;
const SLOTS_W: usize = 7;
const MEM_W: usize = 18;
const OPS_W: usize = 11;
const LAG_W: usize = 8;
const GAP: usize = 3;

/// Which columns fit in `width`, given the widest address on screen.
fn table_cols(width: u16, addr_w: usize) -> TableCols {
    if width < NARROW {
        return TableCols {
            slots: false,
            memory: false,
            ops: false,
            lag: false,
        };
    }
    let base = 2 + addr_w + GAP + ROLE_W; // marker, address, role
    let mut cols = TableCols {
        slots: true,
        memory: true,
        ops: true,
        lag: true,
    };
    let need = |c: &TableCols| {
        base + if c.slots { GAP + SLOTS_W } else { 0 }
            + if c.memory { GAP + MEM_W } else { 0 }
            + if c.ops { GAP + OPS_W } else { 0 }
            + if c.lag { GAP + LAG_W } else { 0 }
    };
    let width = width as usize;
    if need(&cols) > width {
        cols.lag = false;
    }
    if need(&cols) > width {
        cols.ops = false;
    }
    if need(&cols) > width {
        cols.memory = false;
    }
    cols
}

/// The overview: health line, the aggregate tiles, the node table. The tiles
/// keep their size (decision 5); the table is what scrolls.
fn cluster_overview(
    cluster: &crate::state::ClusterDash,
    theme: &Theme,
    area: Rect,
    top: u16,
    buf: &mut Buffer,
) {
    let g = theme.glyphs;
    let mut y = top;
    let tiles = cluster.tiles();

    // The health line: CLUSTER INFO, or why it could not be read.
    let (health_text, health_token) = match cluster.health() {
        Some(Ok(h)) => (h.line(g.get(Glyph::Separator)), alarm_token(h.level())),
        Some(Err(e)) => (
            format!("{} CLUSTER INFO failed: {e}", g.get(Glyph::Deleted)),
            Token::Danger,
        ),
        None => (String::new(), Token::Muted),
    };
    let mut health_text = health_text;
    let mut health_token = health_token;
    if tiles.unreachable > 0 {
        health_text.push_str(&format!(
            " {} {} of {} not answering",
            g.get(Glyph::Separator),
            tiles.unreachable,
            plural(tiles.nodes, "node")
        ));
        health_token = Token::Danger;
    }
    put(
        buf,
        area.x + 1,
        y,
        &truncate(&health_text, area.width.saturating_sub(2) as usize, g),
        theme.style(health_token),
    );
    y += 1;

    let bottom = area.y + area.height;
    if y >= bottom {
        return;
    }
    let nodes = cluster.nodes();
    let table_rows = nodes.len().min(MIN_TABLE_NODES);
    let table_min = if nodes.is_empty() { 0 } else { 1 + table_rows } as u16;
    let cols = tiles_per_row(area.width).max(1);
    let all_tile_rows = TILE_ORDER.len().div_ceil(cols) as u16;
    // Whole tile rows only, as many as leave the table its minimum.
    let room = (bottom - y).saturating_sub(table_min);
    let tile_rows = (room / TILE_STEP).min(all_tile_rows);
    if tile_rows > 0 {
        let tile_area = Rect::new(
            area.x,
            area.y,
            area.width,
            y + tile_rows * TILE_STEP - area.y,
        );
        grid(
            &Tiles::Cluster(&tiles, cluster.ops_history()),
            None,
            theme,
            tile_area,
            y,
            buf,
        );
        y += tile_rows * TILE_STEP;
    }
    if y >= bottom {
        return;
    }
    node_table(cluster, theme, area, y, bottom, buf);
}

fn node_table(
    cluster: &crate::state::ClusterDash,
    theme: &Theme,
    area: Rect,
    top: u16,
    bottom: u16,
    buf: &mut Buffer,
) {
    let g = theme.glyphs;
    let nodes = cluster.nodes();
    let addr_w = nodes
        .iter()
        .map(|n| n.addr.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(12, 24);
    let cols = table_cols(area.width, addr_w);
    let width = area.width as usize;

    // Header.
    let header = |c: &TableCols| {
        let mut h = format!(
            "  {:<addr_w$}{}{:<ROLE_W$}",
            "NODE",
            " ".repeat(GAP),
            "ROLE"
        );
        if c.slots {
            h.push_str(&format!("{}{:>SLOTS_W$}", " ".repeat(GAP), "SLOTS"));
        }
        if c.memory {
            h.push_str(&format!("{}{:<MEM_W$}", " ".repeat(GAP), "MEMORY"));
        }
        if c.ops {
            h.push_str(&format!("{}{:>OPS_W$}", " ".repeat(GAP), "OPS/SEC"));
        }
        if c.lag {
            h.push_str(&format!("{}{:>LAG_W$}", " ".repeat(GAP), "LAG"));
        }
        h
    };
    put(
        buf,
        area.x,
        top,
        &truncate(&header(&cols), width, g),
        theme.style(Token::Muted),
    );

    // The rows, windowed so the cursor row is always on screen.
    let visible = (bottom - top - 1) as usize;
    if visible == 0 {
        return;
    }
    let selected = cluster.selected_index();
    let view = Viewport {
        offset: 0,
        selected,
    }
    .scrolled_to_selection(visible);
    for (i, node) in nodes.iter().enumerate().skip(view.offset).take(visible) {
        let y = top + 1 + (i - view.offset) as u16;
        node_row(theme, area, y, node, i == selected, &cols, addr_w, buf);
    }
    // More rows than fit: say so on the header line's right edge.
    if nodes.len() > visible {
        let more = format!("{}/{}", selected + 1, nodes.len());
        put_right(
            buf,
            area.x,
            top,
            area.width.saturating_sub(1),
            &more,
            theme.style(Token::Muted),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn node_row(
    theme: &Theme,
    area: Rect,
    y: u16,
    node: &ClusterNode,
    selected: bool,
    cols: &TableCols,
    addr_w: usize,
    buf: &mut Buffer,
) {
    let g = theme.glyphs;
    let width = area.width as usize;
    if selected {
        put(
            buf,
            area.x,
            y,
            &" ".repeat(area.width as usize),
            theme.style(Token::Selected),
        );
    }
    // On the selected row the highlight is the colour; alarm hues stay (as the
    // slowlog's duration does), muted and plain text take the selection's.
    let plain = theme.style(if selected {
        Token::Selected
    } else {
        Token::Text
    });
    let muted = theme.style(if selected {
        Token::Selected
    } else {
        Token::Muted
    });
    let marker = if selected { ">" } else { " " };
    let addr_style = if node.failed().is_some() {
        theme.style(Token::Danger)
    } else {
        plain
    };
    put(buf, area.x, y, marker, plain);
    let mut x = area.x + 2;
    let mut cell =
        |text: &str, w: usize, right: bool, style: ratatui::style::Style, buf: &mut Buffer| {
            let t = truncate(text, w + 1, g);
            let pad = w.saturating_sub(t.chars().count());
            let shown = if right {
                format!("{}{t}", " ".repeat(pad))
            } else {
                format!("{t}{}", " ".repeat(pad))
            };
            if (x - area.x) as usize + shown.chars().count() <= width + GAP {
                put(buf, x, y, &shown, style);
            }
            x += (w + GAP) as u16;
        };
    cell(&node.addr, addr_w, false, addr_style, buf);
    cell(node.role.label(), ROLE_W, false, muted, buf);
    // A failed node: the reason replaces the figures. The command and the node
    // are named (R7.4); the full text is also the notification.
    if let Some(detail) = node.failed() {
        let used = 2 + addr_w + GAP + ROLE_W + GAP;
        let room = width.saturating_sub(used + 1);
        let text = format!("{} INFO failed: {detail}", g.get(Glyph::Deleted));
        put(
            buf,
            area.x + used as u16,
            y,
            &truncate(&text, room, g),
            theme.style(Token::Danger),
        );
        return;
    }
    let d = &node.dash;
    if cols.slots {
        let s = if node.role == NodeRole::Primary {
            node.slots.to_string()
        } else {
            g.get(Glyph::Dash).to_string()
        };
        cell(&s, SLOTS_W, true, muted, buf);
    }
    if cols.memory {
        let (text, style) = match d.memory_tile() {
            Some(MemoryTile {
                used_bytes,
                max_bytes: Some(max),
                level,
                ..
            }) => {
                let ratio = used_bytes as f64 / max as f64;
                (
                    format!("[{}] {:>3.0}%", bar(ratio, 8, g), ratio * 100.0),
                    if selected && level == AlarmLevel::Ok {
                        plain
                    } else {
                        theme.style(alarm_token(level))
                    },
                )
            }
            Some(MemoryTile { used_bytes, .. }) => (format_bytes(used_bytes), muted),
            None => (g.get(Glyph::Dash).to_string(), muted),
        };
        cell(&text, MEM_W, false, style, buf);
    }
    if cols.ops {
        let ops = d.ops_history().back().copied();
        let text = match ops {
            Some(n) => thousands(n),
            None => g.get(Glyph::Dash).to_string(),
        };
        cell(&text, OPS_W, true, plain, buf);
    }
    if cols.lag {
        let (text, style) = match d.replication_tile() {
            Some(t) if node.role == NodeRole::Replica => {
                let style = if t.link_status.as_deref() == Some("down") {
                    theme.style(Token::Danger)
                } else {
                    theme.style(alarm_token(t.level))
                };
                match (t.link_status.as_deref(), t.lag_secs) {
                    (Some("down"), _) => ("down".to_string(), style),
                    (_, Some(lag)) => (format!("{lag}s"), style),
                    _ => (g.get(Glyph::Dash).to_string(), muted),
                }
            }
            _ => (g.get(Glyph::Dash).to_string(), muted),
        };
        cell(&text, LAG_W, true, style, buf);
    }
}
