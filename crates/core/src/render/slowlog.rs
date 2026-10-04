//! The Slowlog view (`g s`, R6.4, M3, `docs/plans/m3-slowlog.md`): a
//! full-screen surface, not a third pane (CLAUDE.md: "screen space is a
//! budget, not a canvas") — the same header/list/detail-strip chrome shape
//! every other screen in this app uses (PLAN's "Proves: entries render in a
//! type-aware-consistent frame").
//!
//! There is no liveness here — `SLOWLOG` has no `CLIENT TRACKING`
//! equivalent — but the clock is still read, for the one relative fact this
//! screen does show: each entry's age since it happened (ADR-0011's
//! injected clock, the same one the keys pane's TTL countdown and the
//! Viewer's Read age already read).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::clock::Clock;
use crate::glyphs::Glyph;
use crate::state::State;
use crate::state::slowlog::SlowlogEntry;
use crate::state::value::cell_text;
use crate::theme::{Theme, Token};

use super::keys::truncate;
use super::{put, put_right};

/// Above this, a duration is shown in [`Token::Warn`] — a visual threshold
/// this screen draws for itself, distinct from the server's own
/// `slowlog-log-slower-than`, which is what put the entry in the log at all.
const WARN_DURATION_US: i64 = 100_000;

/// Column layout, computed once per frame from the pane's width — the same
/// discipline `keys::Columns` follows, so a late-arriving value is never
/// what moves a column (not that anything here arrives late: the whole set
/// is fetched in one round trip).
#[derive(Debug, Clone, Copy)]
struct Columns {
    age: Option<(u16, u16)>,
    duration: (u16, u16),
    command: (u16, u16),
    client: Option<(u16, u16)>,
}

const AGE_W: u16 = 10;
const DURATION_W: u16 = 9;
const CLIENT_W: u16 = 22;

/// CLIENT sheds first (below 80 columns), AGE second (below 70) —
/// DURATION and COMMAND are the reason this screen exists, so they are
/// never shed (the same "TTL is the last to go" idea `keys::Columns`
/// follows, applied to what this screen's own reader is hunting for).
fn columns_for(width: u16) -> Columns {
    let inner = width.saturating_sub(2);
    if width >= 80 {
        let command_w = inner.saturating_sub(AGE_W + DURATION_W + CLIENT_W);
        Columns {
            age: Some((1, AGE_W)),
            duration: (1 + AGE_W, DURATION_W),
            command: (1 + AGE_W + DURATION_W, command_w),
            client: Some((1 + AGE_W + DURATION_W + command_w, CLIENT_W)),
        }
    } else if width >= 70 {
        let command_w = inner.saturating_sub(AGE_W + DURATION_W);
        Columns {
            age: Some((1, AGE_W)),
            duration: (1 + AGE_W, DURATION_W),
            command: (1 + AGE_W + DURATION_W, command_w),
            client: None,
        }
    } else {
        let command_w = inner.saturating_sub(DURATION_W);
        Columns {
            age: None,
            duration: (1, DURATION_W),
            command: (1 + DURATION_W, command_w),
            client: None,
        }
    }
}

/// Render the Slowlog view into `area` — the whole body between the title
/// bar and the status/hint bars, exactly as `frame` hands it over.
pub fn render(state: &State, theme: &Theme, clock: &dyn Clock, area: Rect, buf: &mut Buffer) {
    if area.width < 8 || area.height < 3 {
        return;
    }
    let now_ms = clock.now_epoch_ms();
    let mut y = area.y;
    summary_line(state, theme, area, y, buf);
    y += 1;

    if state.slowlog.is_empty() {
        empty_state(state, theme, area, y, buf);
        return;
    }

    let cols = columns_for(area.width);
    column_header(theme, area, cols, y, buf);
    y += 1;

    // A detail strip for the selected entry, reserved only when there is
    // still room for at least one list row underneath it (G7: screen space
    // is a budget) — the same "reserved only while true" rule the keys
    // pane's cap banner and filter line follow.
    let used = y - area.y;
    let detail_rows: u16 = if area.height > used + 3 { 3 } else { 0 };
    let list_bottom = area.y + area.height - detail_rows;
    let list_height = list_bottom.saturating_sub(y) as usize;

    let entries: Vec<&SlowlogEntry> = state.slowlog.ordered().collect();
    // Stateless scrolling: the offset is recomputed from the selection and
    // the viewport height alone every frame, the same
    // `keys::Viewport::scrolled_to_selection` the keys pane persists —
    // except this one is never stored, so a frame stays a pure function of
    // `state.slowlog.selected` with no extra field to keep in step with it
    // (ADR-0011). The ring buffer this reads is small and short-lived
    // enough that this never draws a viewport under a few hundred rows.
    let viewport = super::keys::Viewport {
        offset: 0,
        selected: state.slowlog.selected,
    }
    .scrolled_to_selection(list_height);

    for row in 0..list_height {
        let idx = viewport.offset + row;
        let Some(entry) = entries.get(idx) else {
            break;
        };
        let at = y + row as u16;
        let selected = idx == state.slowlog.selected;
        let ctx = RowCtx {
            theme,
            area,
            cols,
            now_ms,
        };
        entry_row(ctx, at, entry, selected, buf);
    }

    if detail_rows > 0
        && let Some(entry) = state.slowlog.selected_entry()
    {
        detail_strip(theme, area, area.y + area.height - detail_rows, entry, buf);
    }
}

fn summary_line(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let count = state.slowlog.len();
    let noun = if count == 1 { "entry" } else { "entries" };
    let sep = theme.glyphs.get(Glyph::Separator);
    let text = format!(
        "SLOWLOG {sep} {count} {noun} {sep} sort: {}",
        state.slowlog.sort.label()
    );
    put(buf, area.x + 1, y, &text, theme.style(Token::Text));
    if state.slowlog.loading {
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

/// Nothing to show: still fetching, a failed fetch (R7.4's in-view half —
/// the global toast is `Msg::SlowlogFailed`'s other half, in `state.error`),
/// or a real empty ring buffer — a state after a fresh `RESET` (phase B) or
/// simply nothing having crossed `slowlog-log-slower-than` yet. Each says
/// which, rather than drawing the same blank list for three different
/// reasons.
fn empty_state(state: &State, theme: &Theme, area: Rect, y: u16, buf: &mut Buffer) {
    let (text, token) = if let Some(detail) = &state.slowlog.error {
        (
            format!(
                "{} SLOWLOG GET failed: {detail}",
                theme.glyphs.get(Glyph::Deleted)
            ),
            Token::Danger,
        )
    } else if state.slowlog.loading {
        (theme.glyphs.text("⟳ fetching…").into_owned(), Token::Muted)
    } else {
        (
            theme
                .glyphs
                .text(
                    "no slow commands — nothing has crossed the server's threshold since the last reset",
                )
                .into_owned(),
            Token::Muted,
        )
    };
    put(
        buf,
        area.x + 1,
        y,
        &truncate(&text, area.width.saturating_sub(2) as usize, theme.glyphs),
        theme.style(token),
    );
}

fn column_header(theme: &Theme, area: Rect, cols: Columns, y: u16, buf: &mut Buffer) {
    let style = theme.style(Token::Muted);
    if let Some((x, _)) = cols.age {
        put(buf, area.x + x, y, "AGE", style);
    }
    put(buf, area.x + cols.duration.0, y, "DURATION", style);
    put(buf, area.x + cols.command.0, y, "COMMAND", style);
    if let Some((x, _)) = cols.client {
        put(buf, area.x + x, y, "CLIENT", style);
    }
}

/// What every row of one frame's entry list shares, read once per frame
/// rather than passed row by row as loose arguments (the same discipline
/// `keys::RowCtx` follows for the keys pane).
#[derive(Clone, Copy)]
struct RowCtx<'a> {
    theme: &'a Theme,
    area: Rect,
    cols: Columns,
    now_ms: u64,
}

fn entry_row(ctx: RowCtx<'_>, y: u16, entry: &SlowlogEntry, selected: bool, buf: &mut Buffer) {
    let RowCtx {
        theme,
        area,
        cols,
        now_ms,
    } = ctx;
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
    // A duration over the threshold keeps its own hue even on the selected
    // row's highlight — Warn still reads as "this one is slow" the way it
    // does everywhere else colour survives a selection bar (the keys pane's
    // TYPE dot does the same under `Token::Selected`).
    let duration_style = if selected {
        theme.style(Token::Selected)
    } else if entry.duration_us >= WARN_DURATION_US {
        theme.style(Token::Warn)
    } else {
        theme.style(Token::Muted)
    };

    if let Some((x, _)) = cols.age {
        put(
            buf,
            area.x + x,
            y,
            &format_age(entry.timestamp, now_ms),
            muted_style,
        );
    }
    put(
        buf,
        area.x + cols.duration.0,
        y,
        &theme.glyphs.text(&format_duration_us(entry.duration_us)),
        duration_style,
    );
    let command = cell_text(&entry.command);
    put(
        buf,
        area.x + cols.command.0,
        y,
        &truncate(&command, cols.command.1 as usize, theme.glyphs),
        text_style,
    );
    if let Some((x, w)) = cols.client {
        let client = client_label(entry);
        put(
            buf,
            area.x + x,
            y,
            &truncate(&client, w as usize, theme.glyphs),
            muted_style,
        );
    }
}

fn detail_strip(theme: &Theme, area: Rect, y: u16, entry: &SlowlogEntry, buf: &mut Buffer) {
    let w = area.width as usize;
    put(
        buf,
        area.x,
        y,
        &theme.glyphs.get(Glyph::Horizontal).repeat(w),
        theme.style(Token::Border),
    );
    let command = cell_text(&entry.command);
    put(
        buf,
        area.x + 1,
        y + 1,
        &format!(
            "cmd    {}",
            truncate(&command, w.saturating_sub(9), theme.glyphs)
        ),
        theme.style(Token::Text),
    );
    // The exact moment, UTC with date — the age column is coarse and
    // relative on purpose (a scannable "how long ago", not "when exactly"),
    // so the one place a reader needs the precise instant is here, where
    // there is room to spell it out in full (Decision 5,
    // `docs/plans/m3-slowlog.md`).
    let when_and_client = format!(
        "at {}  {}  client {}",
        format_timestamp_utc(entry.timestamp),
        theme.glyphs.get(Glyph::Separator),
        client_label(entry)
    );
    put(
        buf,
        area.x + 1,
        y + 2,
        &truncate(&when_and_client, w.saturating_sub(1), theme.glyphs),
        theme.style(Token::Muted),
    );
}

/// The client name if the connection was named (`CLIENT SETNAME`), else its
/// address — whichever actually identifies something to the reader, escaped
/// the same way any other collection member's bytes are (`cell_text`,
/// reused from the Viewer rather than a second escaping rule for this
/// screen).
fn client_label(entry: &SlowlogEntry) -> String {
    if entry.client_name.is_empty() {
        cell_text(&entry.client_addr)
    } else {
        format!(
            "{} ({})",
            cell_text(&entry.client_name),
            cell_text(&entry.client_addr)
        )
    }
}

/// Age relative to the injected clock, at the coarsest useful precision —
/// `12s ago`, `3h ago`, `2d ago` (Decision 5, `docs/plans/m3-slowlog.md`: a
/// bare `HH:MM:SS` is ambiguous for an entry days old). Reuses
/// `keys::format_ttl`'s own s/m/h/d ladder rather than a second one to keep
/// in step with it — elapsed time and a TTL countdown are the same
/// magnitude problem, read in opposite directions, and `format_ttl` never
/// collapses a figure to a unit that hides it, which this wants exactly as
/// much as a countdown does.
fn format_age(entry_timestamp: i64, now_ms: u64) -> String {
    let now_s = (now_ms / 1000) as i64;
    let elapsed = (now_s - entry_timestamp).clamp(0, i32::MAX as i64) as i32;
    format!("{} ago", super::keys::format_ttl(elapsed))
}

/// The exact moment, UTC with date (`2026-09-27 01:02:03 UTC`) — the detail
/// strip's own precision, where the age column is deliberately coarse and
/// relative instead (Decision 5). No timezone conversion is available to the
/// core (ADR-0011: no environment access beyond the injected clock), so this
/// says UTC explicitly rather than presenting a local time it cannot
/// actually compute.
///
/// The date math is a hand-rolled `civil_from_days` (Howard Hinnant's
/// `chrono`-compatible day-count/civil-date algorithm — public domain,
/// widely reused, e.g. by the `time` and `chrono` crates' own internals) —
/// not a new dependency, since the core crate carries none beyond `ratatui`/
/// `serde` (the `boundary` CI job forbids `tokio`/`fred`/`crossterm`, and a
/// general-purpose date crate is more than one screen's timestamp needs).
fn format_timestamp_utc(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let secs_in_day = epoch_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let h = secs_in_day / 3600;
    let min = (secs_in_day % 3600) / 60;
    let s = secs_in_day % 60;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02} UTC")
}

/// Floor division — `i64`'s `/` truncates toward zero, which is wrong for a
/// negative day count (a moment before 1970), so this is what
/// `civil_from_days` actually needs throughout.
fn div_floor(a: i64, b: i64) -> i64 {
    let q = a / b;
    let r = a % b;
    if r != 0 && (r < 0) != (b < 0) {
        q - 1
    } else {
        q
    }
}

/// Days-since-epoch to a proleptic Gregorian (year, month, day) — Howard
/// Hinnant's `civil_from_days`, verified here against known dates rather
/// than trusted blind.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = div_floor(z, 146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Microseconds, at the coarsest useful precision — mirrors
/// `keys::format_ttl`'s "never collapse to a unit that hides the figure a
/// reader needs" rule, one duration shape over.
fn format_duration_us(us: i64) -> String {
    if us < 1_000 {
        format!("{us}µs")
    } else if us < 1_000_000 {
        format!("{:.1}ms", us as f64 / 1_000.0)
    } else {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_is_relative_to_the_injected_clock_at_the_coarsest_useful_precision() {
        let now_ms = 1_700_000_000_000u64; // 2023-11-14 22:13:20 UTC
        assert_eq!(format_age(1_700_000_000 - 12, now_ms), "12s ago");
        assert_eq!(format_age(1_700_000_000 - 3 * 3_600, now_ms), "3h ago");
        assert_eq!(format_age(1_700_000_000 - 2 * 86_400, now_ms), "2d ago");
    }

    #[test]
    fn age_never_goes_negative_for_a_clock_reading_before_the_entry() {
        // A clock skew or a stale fixture must not print "ago" over a
        // negative figure — clamped to "just happened" instead.
        let now_ms = 1_700_000_000_000u64;
        assert_eq!(format_age(1_700_000_100, now_ms), "0s ago");
    }

    // civil_from_days, checked against `date -u -r <epoch> +%Y-%m-%d`
    // ground truth rather than trusted blind.
    #[test]
    fn timestamp_matches_known_dates() {
        assert_eq!(format_timestamp_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_timestamp_utc(3_723), "1970-01-01 01:02:03 UTC");
        assert_eq!(
            format_timestamp_utc(1_700_000_000),
            "2023-11-14 22:13:20 UTC"
        );
    }

    #[test]
    fn duration_formats_at_the_coarsest_useful_precision() {
        assert_eq!(format_duration_us(42), "42µs");
        assert_eq!(format_duration_us(1_234), "1.2ms");
        assert_eq!(format_duration_us(1_500_000), "1.50s");
    }

    #[test]
    fn columns_shed_client_first_then_age() {
        let full = columns_for(100);
        assert!(full.client.is_some() && full.age.is_some());
        let no_client = columns_for(75);
        assert!(no_client.client.is_none() && no_client.age.is_some());
        let narrow = columns_for(50);
        assert!(narrow.client.is_none() && narrow.age.is_none());
    }
}
