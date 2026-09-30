//! The Pub/Sub view (`g p`, R6.2, M3 task 5, `docs/plans/m3-pubsub.md`): a
//! full-screen surface, not a third pane (CLAUDE.md: "screen space is a
//! budget, not a canvas") — the subscription-chip strip, the feed status
//! line, the filter line, the two-plus-time-column tail, and a detail strip
//! for the selected message.
//!
//! **Distinct from Monitor's layout** (PLAN's own "Proves" for this row,
//! `docs/plans/m3-pubsub.md`'s "Why Pub/Sub is not Monitor"): the chip strip
//! has no Monitor analogue at all — Monitor has no "configure before you see
//! anything" step — and the tail is at minimum two columns (channel,
//! payload) where Monitor's is one (raw command text).
//!
//! No persistent warning banner: subscribing costs only what the reader
//! chose to subscribe to, never a server-wide cost the way `MONITOR` is
//! (decision 7) — so, unlike `render::monitor`, this body has no banner row
//! at all.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::state::pubsub::PubSubMessage;
use crate::state::value::{JsonValue, cell_text, looks_like_json};
use crate::state::view::{FilterMode, matches};
use crate::state::{FeedStatus, PubSubFocus, State};
use crate::theme::{Theme, Token};

use super::keys::{Viewport, truncate};
use super::{put, put_right};

const TIME_W: u16 = 13;
const CHANNEL_W: u16 = 22;

#[derive(Debug, Clone, Copy)]
struct Columns {
    time: Option<(u16, u16)>,
    channel: Option<(u16, u16)>,
    payload: (u16, u16),
}

/// TIME sheds first (below 80 columns), CHANNEL second (below 70) — PAYLOAD
/// is never shed. Below 70, CHANNEL is folded into the payload column as a
/// `[channel] payload` prefix rather than lost outright: this screen's own
/// reason to exist (decision 1's "distinct from Monitor's layout") is
/// exactly the channel dimension, so narrowing must repack it, not drop it,
/// the way `render::monitor::columns_for` never sheds COMMAND.
fn columns_for(width: u16) -> Columns {
    let inner = width.saturating_sub(2);
    if width >= 80 {
        let payload_w = inner.saturating_sub(TIME_W + CHANNEL_W);
        Columns {
            time: Some((1, TIME_W)),
            channel: Some((1 + TIME_W, CHANNEL_W)),
            payload: (1 + TIME_W + CHANNEL_W, payload_w),
        }
    } else if width >= 70 {
        let payload_w = inner.saturating_sub(CHANNEL_W);
        Columns {
            time: None,
            channel: Some((1, CHANNEL_W)),
            payload: (1 + CHANNEL_W, payload_w),
        }
    } else {
        Columns {
            time: None,
            channel: None,
            payload: (1, inner),
        }
    }
}

/// Render the Pub/Sub view into `area` — the whole body between the title
/// bar and the status/hint bars, exactly as `frame` hands it over.
pub fn render(state: &State, theme: &Theme, area: Rect, buf: &mut Buffer) {
    if area.width < 8 || area.height < 3 {
        return;
    }
    let mut y = area.y;
    strip_line(state, theme, area, y, buf);
    y += 1;
    if y >= area.y + area.height {
        return;
    }
    status_line(state, theme, area, y, buf);
    y += 1;

    if state.filtering || !state.pubsub.filter.is_empty() {
        if y >= area.y + area.height {
            return;
        }
        filter_line(state, theme, area, y, buf);
        y += 1;
    }
    if y >= area.y + area.height {
        return;
    }

    if state.pubsub.is_empty() {
        empty_state(state, theme, area, y, buf);
        return;
    }

    let cols = columns_for(area.width);
    column_header(theme, area, cols, y, buf);
    y += 1;
    if y >= area.y + area.height {
        return;
    }

    // A detail strip for the selected message, reserved only when there is
    // still room for at least one list row underneath it (G7: screen space
    // is a budget) — the same rule `render::slowlog`'s own detail strip
    // follows, sized generously here since a JSON payload wants more than
    // the two fixed lines Slowlog's own entry needs.
    let space_below = (area.y + area.height).saturating_sub(y);
    let detail_rows: u16 = if space_below >= 4 {
        (space_below - 1).min(8)
    } else {
        0
    };
    let list_bottom = area.y + area.height - detail_rows;
    let list_height = list_bottom.saturating_sub(y) as usize;

    let shown = filtered_messages(state);
    if shown.is_empty() {
        put(
            buf,
            area.x + 1,
            y,
            "no messages match the filter",
            theme.style(Token::Muted),
        );
        return;
    }

    let selected_in_shown = shown
        .iter()
        .position(|&(i, _)| i == state.pubsub.selected())
        .unwrap_or(shown.len().saturating_sub(1));
    let viewport = Viewport {
        offset: 0,
        selected: selected_in_shown,
    }
    .scrolled_to_selection(list_height);

    for row in 0..list_height {
        let idx = viewport.offset + row;
        let Some(&(_, msg)) = shown.get(idx) else {
            break;
        };
        let at = y + row as u16;
        let selected = idx == selected_in_shown;
        message_row(theme, area, cols, at, msg, selected, buf);
    }

    if detail_rows > 0
        && let Some(msg) = state.pubsub.selected_message()
    {
        detail_strip(
            state,
            theme,
            area,
            area.y + area.height - detail_rows,
            msg,
            detail_rows,
            buf,
        );
    }
}

/// The messages currently worth showing, paired with their position in
/// `PubSubState`'s own buffer — filtered by `PubSubState::filter` against
/// channel *or* payload (decision 8), otherwise every message. Recomputed at
/// render time, never cached: filtering narrows the *display*, never the
/// buffer.
fn filtered_messages(state: &State) -> Vec<(usize, &PubSubMessage)> {
    let msgs = state.pubsub.messages();
    if state.pubsub.filter.is_empty() {
        msgs.iter().enumerate().collect()
    } else {
        msgs.iter()
            .enumerate()
            .filter(|(_, m)| {
                matches(&m.channel, &state.pubsub.filter, FilterMode::Glob)
                    || matches(&m.payload, &state.pubsub.filter, FilterMode::Glob)
            })
            .collect()
    }
}

/// The subscription-chip strip (decision 1): `⟡`/`▶` marks whether the strip
/// currently has focus, each subscription is one `[name]` chip (a pattern's
/// carries a trailing `⁎`), the selected chip (while the strip is focused)
/// is highlighted, and `a add` is always shown at the right as the way to
/// add another — this is Pub/Sub's own "configure before you see anything"
/// step, which Monitor has no equivalent of.
fn strip_line(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    if state.pubsub.adding {
        let x = put(buf, area.x + 1, y, "add: ", theme.style(Token::Warn));
        put(
            buf,
            x,
            y,
            &format!("{}▏", state.pubsub.input),
            theme.style(Token::Text),
        );
        return;
    }

    let strip_focused = state.pubsub.focus == PubSubFocus::Strip;
    let marker_style = theme.style(if strip_focused {
        Token::BorderFocus
    } else {
        Token::Muted
    });
    let mut x = put(
        buf,
        area.x + 1,
        y,
        if strip_focused { "▶ " } else { "⟡ " },
        marker_style,
    );

    let right_budget = "a add".chars().count() as u16 + 2;
    let chip_limit = area.x + area.width.saturating_sub(right_budget);

    if state.pubsub.subscriptions.is_empty() {
        put(buf, x, y, "no subscriptions", theme.style(Token::Muted));
    } else {
        for (i, sub) in state.pubsub.subscriptions.iter().enumerate() {
            if x >= chip_limit {
                put(buf, x, y, "…", theme.style(Token::Muted));
                break;
            }
            let selected = strip_focused && i == state.pubsub.selected_chip;
            let style = theme.style(if selected {
                Token::Selected
            } else {
                Token::Text
            });
            let label = if sub.is_pattern() {
                format!("[{} ⁎]", sub.name())
            } else {
                format!("[{}]", sub.name())
            };
            x = put(buf, x, y, &label, style);
            x = put(buf, x, y, " ", theme.style(Token::Text));
        }
    }

    put_right(
        buf,
        area.x,
        y,
        area.width.saturating_sub(1),
        "a add",
        theme.style(Token::Muted),
    );
}

/// The feed's own status, plus (while the tail has focus) pause/following,
/// and how many messages are held — `render::monitor::status_line`,
/// mirrored.
fn status_line(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let (status_text, status_token) = match &state.pubsub.status {
        FeedStatus::Idle => ("not subscribed".to_string(), Token::Muted),
        FeedStatus::Connecting => ("connecting…".to_string(), Token::Muted),
        FeedStatus::Open => ("● live".to_string(), Token::Ok),
        FeedStatus::Closed { reason } => (
            match reason {
                Some(r) => format!("feed closed: {r}"),
                None => "feed closed".to_string(),
            },
            Token::Danger,
        ),
    };
    let mut text = status_text;
    if state.pubsub.focus == PubSubFocus::Tail {
        if state.pubsub.paused {
            text.push_str(&format!(
                " · paused · {} skipped",
                state.pubsub.dropped_while_paused
            ));
        } else if !state.pubsub.following && !state.pubsub.is_empty() {
            // An empty tail has nothing to follow or to have moved off.
            text.push_str(" · following off · End to resume");
        }
    }

    let count = state.pubsub.len();
    let noun = if count == 1 { "message" } else { "messages" };
    let readout = format!("{count} {noun}");
    let left_budget = (area.width as usize)
        .saturating_sub(2)
        .saturating_sub(readout.chars().count() + 2);
    put(
        buf,
        area.x + 1,
        y,
        &truncate(&text, left_budget),
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
    let cursor = if state.filtering { "▏" } else { "" };
    put(
        buf,
        x,
        y,
        &format!("{}{cursor}", state.pubsub.filter),
        theme.style(Token::Text),
    );
    if !state.pubsub.filter.is_empty() {
        let shown = filtered_messages(state).len();
        let readout = format!("{shown} of {}", state.pubsub.len());
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
    let text = if state.pubsub.subscriptions.is_empty() {
        "no subscriptions yet — a to add one"
    } else {
        match &state.pubsub.status {
            FeedStatus::Idle | FeedStatus::Connecting => "connecting…",
            FeedStatus::Open => "nothing has arrived yet",
            FeedStatus::Closed { .. } => "feed closed — nothing arrived before it did",
        }
    };
    put(
        buf,
        area.x + 1,
        y,
        &truncate(text, area.width.saturating_sub(2) as usize),
        theme.style(Token::Muted),
    );
}

fn column_header(theme: &Theme, area: Rect, cols: Columns, y: u16, buf: &mut Buffer) {
    let style = theme.style(Token::Muted);
    if let Some((x, _)) = cols.time {
        put(buf, area.x + x, y, "TIME", style);
    }
    if let Some((x, _)) = cols.channel {
        put(buf, area.x + x, y, "CHANNEL", style);
    }
    put(buf, area.x + cols.payload.0, y, "PAYLOAD", style);
}

/// `at_ms` (receipt time, ADR-0011) to `HH:MM:SS.mmm` UTC — decision 9:
/// Pub/Sub messages carry no server-side timestamp of their own, unlike
/// `MONITOR`'s lines, so this is the shell's receipt time and nothing else.
fn format_time_ms(at_ms: u64) -> String {
    let ms = at_ms % 1000;
    let total_secs = at_ms / 1000;
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

fn message_row(
    theme: &Theme,
    area: Rect,
    cols: Columns,
    y: u16,
    msg: &PubSubMessage,
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
    if let Some((x, w)) = cols.time {
        put(
            buf,
            area.x + x,
            y,
            &truncate(&format_time_ms(msg.at_ms), w as usize),
            muted_style,
        );
    }
    // Decision 8: shown with the Viewer's own byte escaping, never raw.
    let channel = cell_text(&msg.channel);
    if let Some((x, w)) = cols.channel {
        put(
            buf,
            area.x + x,
            y,
            &truncate(&channel, w as usize),
            text_style,
        );
    }
    let payload = cell_text(&msg.payload);
    let payload = if msg.truncated {
        format!("{payload} […truncated]")
    } else {
        payload
    };
    // Below 70 columns CHANNEL has no column of its own — folded into the
    // payload text instead of lost (`columns_for`'s own doc comment).
    let payload_text = if cols.channel.is_none() {
        format!("[{channel}] {payload}")
    } else {
        payload
    };
    put(
        buf,
        area.x + cols.payload.0,
        y,
        &truncate(&payload_text, cols.payload.1 as usize),
        text_style,
    );
}

/// The selected message's own detail — channel, `via` (hedged when more
/// than one subscribed pattern could have produced it, M3 task 5 (c)), and
/// the full payload, pretty-printed when it parses as JSON — the same
/// shape `render::slowlog`'s own detail strip uses (a border line, then
/// content), and the Viewer's own byte-escaping (`cell_text`) throughout.
fn detail_strip(
    state: &State,
    theme: &Theme,
    area: Rect,
    y: u16,
    msg: &PubSubMessage,
    rows: u16,
    buf: &mut Buffer,
) {
    let w = area.width as usize;
    put(buf, area.x, y, &"─".repeat(w), theme.style(Token::Border));

    let channel = cell_text(&msg.channel);
    let mut head = format!("channel {channel}");
    if let Some(via) = &msg.via {
        let via_text = cell_text(via);
        if msg.ambiguous_via(&state.pubsub.subscriptions) {
            // M3 task 5 (c): Redis delivers one `pmessage` per matching
            // pattern, and fred's public API does not say which one
            // produced this delivery when more than one subscribed pattern
            // matches — hedged rather than asserted.
            head.push_str(&format!(
                "  ·  via {via_text} (or another matching pattern)"
            ));
        } else {
            head.push_str(&format!("  ·  via {via_text}"));
        }
    }
    put(
        buf,
        area.x + 1,
        y + 1,
        &truncate(&head, w.saturating_sub(1)),
        theme.style(Token::Text),
    );

    let payload_text = cell_text(&msg.payload);
    let lines: Vec<String> = if looks_like_json(&payload_text) {
        JsonValue::parse(&payload_text).lines
    } else {
        payload_text.lines().map(str::to_string).collect()
    };
    // `rows` includes the border and the head line above.
    let available = rows.saturating_sub(2) as usize;
    if available == 0 {
        return;
    }
    if lines.len() <= available {
        for (i, line) in lines.iter().enumerate() {
            put(
                buf,
                area.x + 1,
                y + 2 + i as u16,
                &truncate(line, w.saturating_sub(1)),
                theme.style(Token::Text),
            );
        }
    } else {
        // Leave the last visible row for a "more" indicator rather than
        // silently cutting a JSON body off mid-structure.
        let shown = available.saturating_sub(1);
        for (i, line) in lines.iter().take(shown).enumerate() {
            put(
                buf,
                area.x + 1,
                y + 2 + i as u16,
                &truncate(line, w.saturating_sub(1)),
                theme.style(Token::Text),
            );
        }
        let more = format!("… (+{} more lines)", lines.len() - shown);
        put(
            buf,
            area.x + 1,
            y + 2 + shown as u16,
            &truncate(&more, w.saturating_sub(1)),
            theme.style(Token::Muted),
        );
    }
}
