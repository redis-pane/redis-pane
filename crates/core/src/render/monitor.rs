//! The Monitor view (`g m`, R6.1, M3 phase B, `docs/plans/m3-monitor.md`): a
//! full-screen surface, not a third pane (CLAUDE.md: "screen space is a
//! budget, not a canvas") — the persistent cost banner, the feed status/
//! pause/following readout, the filter line, and the tail itself.
//!
//! The banner is decision 10's own requirement: present at every width down
//! to the single-pane floor, and never dismissible short of leaving the
//! view — drawn first, unconditionally, before anything else in this body
//! is laid out.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::glyphs::Glyph;
use crate::state::monitor::MonitorLine;
use crate::state::value::cell_text;
use crate::state::view::{FilterMode, matches};
use crate::state::{FeedStatus, State};
use crate::theme::{Theme, Token};

use super::keys::{Viewport, truncate};
use super::{put, put_right};

const TIME_W: u16 = 13;
const DB_W: u16 = 4;
const CLIENT_W: u16 = 22;

#[derive(Debug, Clone, Copy)]
struct Columns {
    time: Option<(u16, u16)>,
    db: Option<(u16, u16)>,
    client: Option<(u16, u16)>,
    command: (u16, u16),
}

/// CLIENT sheds first, then DB, then TIME — COMMAND is never shed, the same
/// "the reason this screen exists is never the first thing cut" rule
/// `render::slowlog::columns_for` follows for DURATION/COMMAND there.
fn columns_for(width: u16) -> Columns {
    let inner = width.saturating_sub(2);
    if width >= 90 {
        let command_w = inner.saturating_sub(TIME_W + DB_W + CLIENT_W);
        Columns {
            time: Some((1, TIME_W)),
            db: Some((1 + TIME_W, DB_W)),
            client: Some((1 + TIME_W + DB_W, CLIENT_W)),
            command: (1 + TIME_W + DB_W + CLIENT_W, command_w),
        }
    } else if width >= 70 {
        let command_w = inner.saturating_sub(TIME_W + DB_W);
        Columns {
            time: Some((1, TIME_W)),
            db: Some((1 + TIME_W, DB_W)),
            client: None,
            command: (1 + TIME_W + DB_W, command_w),
        }
    } else {
        let command_w = inner.saturating_sub(TIME_W);
        Columns {
            time: Some((1, TIME_W)),
            db: None,
            client: None,
            command: (1 + TIME_W, command_w),
        }
    }
}

/// Render the Monitor view into `area` — the whole body between the title
/// bar and the status/hint bars, exactly as `frame` hands it over.
pub fn render(state: &State, theme: &Theme, area: Rect, buf: &mut Buffer) {
    if area.width < 8 || area.height < 3 {
        return;
    }
    if state.on_cluster() {
        super::cluster_notice(
            theme,
            area,
            "MONITOR",
            "⚠ Monitor is per node on a Cluster — coming in M5 (task 8)",
            buf,
        );
        return;
    }
    let mut y = area.y;
    banner(state, theme, area, y, buf);
    y += 1;
    if y >= area.y + area.height {
        return;
    }
    status_line(state, theme, area, y, buf);
    y += 1;

    if state.filtering || !state.monitor.filter.is_empty() {
        if y >= area.y + area.height {
            return;
        }
        filter_line(state, theme, area, y, buf);
        y += 1;
    }

    if y >= area.y + area.height {
        return;
    }

    if state.monitor.is_empty() {
        empty_state(state, theme, area, y, buf);
        return;
    }

    let cols = columns_for(area.width);
    column_header(theme, area, cols, y, buf);
    y += 1;
    if y >= area.y + area.height {
        return;
    }

    let list_height = (area.y + area.height).saturating_sub(y) as usize;
    let shown = filtered_lines(state);
    if shown.is_empty() {
        put(
            buf,
            area.x + 1,
            y,
            "no lines match the filter",
            theme.style(Token::Muted),
        );
        return;
    }

    // `shown`'s own index of the selected line, not `state.monitor.selected`
    // directly — the filter narrows what's *displayed* without touching
    // `MonitorState::selected`'s position in the unfiltered buffer (decision
    // 6), so a viewport built over the filtered subset needs the selection
    // re-expressed in that subset's own coordinates.
    let selected_in_shown = shown
        .iter()
        .position(|&(i, _)| i == state.monitor.selected())
        .unwrap_or(shown.len().saturating_sub(1));
    let viewport = Viewport {
        offset: 0,
        selected: selected_in_shown,
    }
    .scrolled_to_selection(list_height);

    for row in 0..list_height {
        let idx = viewport.offset + row;
        let Some(&(_, line)) = shown.get(idx) else {
            break;
        };
        let at = y + row as u16;
        let selected = idx == selected_in_shown;
        line_row(theme, area, cols, at, line, selected, buf);
    }
}

/// The lines currently worth showing, paired with their position in
/// `MonitorState`'s own buffer (needed to re-locate the selection above) —
/// filtered by `MonitorState::filter` if one is set, otherwise every line
/// (decision 6: filtering narrows the *display*, never the buffer, so this
/// is recomputed at render time rather than cached anywhere in `State`).
fn filtered_lines(state: &State) -> Vec<(usize, &MonitorLine)> {
    let lines = state.monitor.lines();
    if state.monitor.filter.is_empty() {
        lines.iter().enumerate().collect()
    } else {
        lines
            .iter()
            .enumerate()
            .filter(|(_, l)| matches(l.raw.as_bytes(), &state.monitor.filter, FilterMode::Glob))
            .collect()
    }
}

/// Decision 10: present whenever the view is open, at every width down to
/// the single-pane floor, and never dismissible short of leaving the view —
/// drawn unconditionally, first, before anything else in the body.
///
/// "Impossible to miss" also means never cut off mid-sentence: an `…`
/// swallowing half the warning at the 80-column floor (R7.1) is exactly the
/// half-missed banner decision 10 exists to rule out. So this picks the
/// longest of two fixed wordings that actually fits `area.width`, rather
/// than truncating one long sentence — never `truncate`, here.
///
/// Once the feed has closed the warning would be false — nothing is running
/// and the server pays nothing — so the same line says so instead, muted,
/// keeping the layout still rather than collapsing a row under the reader.
const BANNER_FULL: &str =
    "⚠ MONITOR is running — the server pays for every command it streams here";
const BANNER_SHORT: &str = "⚠ MONITOR running — costs the server";
const STOPPED_FULL: &str = "MONITOR stopped — the server is no longer streaming to this view";
const STOPPED_SHORT: &str = "MONITOR stopped";

fn banner(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let stopped = matches!(state.monitor.status, FeedStatus::Closed { .. });
    let (full, short, token) = if stopped {
        (STOPPED_FULL, STOPPED_SHORT, Token::Muted)
    } else {
        (BANNER_FULL, BANNER_SHORT, Token::Warn)
    };
    let usable = area.width.saturating_sub(1) as usize;
    let text = if full.chars().count() <= usable {
        full
    } else {
        short
    };
    put(
        buf,
        area.x,
        y,
        &" ".repeat(area.width as usize),
        theme.style(token),
    );
    put(
        buf,
        area.x + 1,
        y,
        &theme.glyphs.text(text),
        theme.style(token),
    );
}

/// The feed's own status (decision 9), plus pause/following (decisions 5, 7)
/// and how many lines are held.
fn status_line(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let (status_text, status_token) = match &state.monitor.status {
        FeedStatus::Idle | FeedStatus::Connecting => {
            (theme.glyphs.text("connecting…").into_owned(), Token::Muted)
        }
        FeedStatus::Open => (
            theme.glyphs.get(Glyph::Live).to_string() + " live",
            Token::Ok,
        ),
        FeedStatus::Closed { reason } => (
            match reason {
                Some(r) => format!("feed closed: {r}"),
                None => "feed closed".to_string(),
            },
            Token::Danger,
        ),
    };
    let mut text = status_text;
    if state.monitor.paused {
        text.push_str(&format!(
            " {} paused {} {} skipped",
            theme.glyphs.get(Glyph::Separator),
            theme.glyphs.get(Glyph::Separator),
            state.monitor.dropped_while_paused
        ));
    } else if !state.monitor.following {
        text.push_str(&theme.glyphs.text(" · following off · End to resume"));
    }

    let count = state.monitor.len();
    let noun = if count == 1 { "line" } else { "lines" };
    let readout = format!("{count} {noun}");
    // Reserve room for the right-aligned readout so a long status (a feed
    // closed with a real reason, say) truncates before it runs into it,
    // rather than overlapping it with no gap.
    let left_budget = (area.width as usize)
        .saturating_sub(2)
        .saturating_sub(readout.chars().count() + 2);
    put(
        buf,
        area.x + 1,
        y,
        &truncate(&text, left_budget, theme.glyphs),
        theme.style(status_token),
    );
    put_right(
        buf,
        area.x,
        y,
        area.width.saturating_sub(1),
        &readout,
        theme.style(Token::Muted),
    );
}

fn filter_line(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let x = put(buf, area.x + 1, y, "/ ", theme.style(Token::Warn));
    let cursor = if state.filtering {
        theme.glyphs.get(Glyph::Cursor)
    } else {
        ""
    };
    put(
        buf,
        x,
        y,
        &format!("{}{cursor}", state.monitor.filter),
        theme.style(Token::Text),
    );
    if !state.monitor.filter.is_empty() {
        let shown = filtered_lines(state).len();
        let readout = format!("{shown} of {}", state.monitor.len());
        put_right(
            buf,
            area.x,
            y,
            area.width.saturating_sub(1),
            &readout,
            theme.style(Token::Muted),
        );
    }
}

fn empty_state(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let text = match &state.monitor.status {
        FeedStatus::Idle | FeedStatus::Connecting => "connecting…",
        FeedStatus::Open => "nothing has arrived yet",
        FeedStatus::Closed { .. } => "feed closed — nothing arrived before it did",
    };
    let text = &*theme.glyphs.text(text);
    put(
        buf,
        area.x + 1,
        y,
        &truncate(text, area.width.saturating_sub(2) as usize, theme.glyphs),
        theme.style(Token::Muted),
    );
}

fn column_header(theme: &Theme, area: Rect, cols: Columns, y: u16, buf: &mut Buffer) {
    let style = theme.style(Token::Muted);
    if let Some((x, _)) = cols.time {
        put(buf, area.x + x, y, "TIME", style);
    }
    if let Some((x, _)) = cols.db {
        put(buf, area.x + x, y, "DB", style);
    }
    if let Some((x, _)) = cols.client {
        put(buf, area.x + x, y, "CLIENT", style);
    }
    put(buf, area.x + cols.command.0, y, "COMMAND", style);
}

fn line_row(
    theme: &Theme,
    area: Rect,
    cols: Columns,
    y: u16,
    line: &MonitorLine,
    selected: bool,
    buf: &mut Buffer,
) {
    if selected {
        put(
            buf,
            area.x,
            y,
            &" ".repeat(area.width as usize),
            theme.style(Token::Selected),
        );
    }
    let text_style = theme.style(if selected {
        Token::Selected
    } else {
        Token::Text
    });
    let muted_style = theme.style(if selected {
        Token::Selected
    } else {
        Token::Muted
    });
    let parsed = line.columns();
    if let Some((x, w)) = cols.time {
        let t = parsed
            .time
            .as_deref()
            .unwrap_or(theme.glyphs.get(Glyph::Dash));
        put(
            buf,
            area.x + x,
            y,
            &truncate(t, w as usize, theme.glyphs),
            muted_style,
        );
    }
    if let Some((x, w)) = cols.db {
        put(
            buf,
            area.x + x,
            y,
            &truncate(parsed.db, w as usize, theme.glyphs),
            muted_style,
        );
    }
    if let Some((x, w)) = cols.client {
        put(
            buf,
            area.x + x,
            y,
            &truncate(parsed.client, w as usize, theme.glyphs),
            muted_style,
        );
    }
    // Decision 8: shown with the Viewer's own byte escaping, never raw.
    let command = cell_text(parsed.command.as_bytes());
    let command = if line.truncated {
        format!("{command} [{}truncated]", theme.glyphs.get(Glyph::Ellipsis))
    } else {
        command
    };
    put(
        buf,
        area.x + cols.command.0,
        y,
        &truncate(&command, cols.command.1 as usize, theme.glyphs),
        text_style,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_shed_client_then_db_never_command() {
        let full = columns_for(100);
        assert!(full.client.is_some() && full.db.is_some());
        let no_client = columns_for(75);
        assert!(no_client.client.is_none() && no_client.db.is_some());
        let narrow = columns_for(50);
        assert!(narrow.client.is_none() && narrow.db.is_none());
        assert!(narrow.command.1 > 0);
    }
}
